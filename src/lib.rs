/*
 * Copyright (C) 2026 [Your Name/Company]
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod editor;

const TIME_MS_MIN: f32 = 1.0;
const TIME_MS_MAX: f32 = 1500.0;
const NUM_SYNC_STEPS: f32 = 18.0;

pub struct TapeDelay {
    params: Arc<TapeParams>,

    // DSP State
    delay_buffer_l: Vec<f32>,
    delay_buffer_r: Vec<f32>,
    write_pos: usize,
    sample_rate: f32,
    current_delay_samples: f32,

    // --- NEW FIELDS FOR TAPE MOJO --- //

    // 1. LFO Phase (0.0 to 2*PI)
    // Needs to persist so the wobble is smooth across buffers
    lfo_phase: f32,

    // 2. Filter States
    // These hold the "previous sample" value for the Low Pass filters
    lp_state_l: f32,
    lp_state_r: f32,

    // 3. Random Seed
    // Needs to persist so the noise doesn't repeat identical patterns every buffer
    rng_seed: u32,

    // Tracks the current "health" of the signal (0.0 to 1.0)
    // We smooth this so volume dips are wobbly, not instant.
    dropout_smoother: f32,
    dropout_timer: f32, // Tracks how much longer the tape stays "unplugged"

    // Corrosion (Erosion-style phase-modulated delay) State
    corrosion_buf_l: Vec<f32>,
    corrosion_buf_r: Vec<f32>,
    corrosion_write: usize,
    corrosion_sine_phase: f32,
    // Bandpass filter states for noise modulator (two 1-pole stages each channel)
    corrosion_bp_l: [f32; 2],
    corrosion_bp_r: [f32; 2],
    // Small LCG for corrosion noise generation
    corrosion_rng: u32,

    /// The decay factor for a single sample
    meter_decay_per_sample: f32,
    peak_meter_l: Arc<AtomicF32>,
    peak_meter_r: Arc<AtomicF32>,

    crackle_integrator_l: f32,
    crackle_integrator_r: f32,
    crackle_hp_l: f32,
    crackle_hp_r: f32,
    was_distortion_mode: bool,
}

#[derive(Params)]
struct TapeParams {
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,

    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "time"]
    pub delay_time_ms: FloatParam,
    #[persist = "time_sync_state"]
    pub is_sync_active: Arc<AtomicBool>,
    #[id = "time_sync"]
    pub time_sync: BoolParam,
    #[id = "broken_tape"]
    pub broken_tape: BoolParam,
    #[id = "distortion_mode"]
    pub distortion_mode: BoolParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    pub ghost_zero: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "crackle"]
    pub crackle: FloatParam,
    #[id = "stereo_width"]
    pub stereo_width: FloatParam,
}

impl Default for TapeParams {
    fn default() -> Self {
        let sync_default = true;
        let is_time_sync_active = Arc::new(AtomicBool::new(sync_default));

        // Clone it for the closure
        let time_sync_flag_for_formatter = is_time_sync_active.clone();
        let time_sync_flag_for_callback = is_time_sync_active.clone();

        // Create the shared memory flag
        let is_tape_broken = Arc::new(AtomicBool::new(false));

        // Clone it for the closure
        let tape_broken_flag_for_callback = is_tape_broken.clone();

        // Create distortion mode flag for formatters
        let is_distortion_mode = Arc::new(AtomicBool::new(false));
        let distortion_flag_for_callback = is_distortion_mode.clone();
        let distortion_flag_for_feedback_formatter = is_distortion_mode.clone();
        let distortion_flag_for_mix_formatter = is_distortion_mode.clone();

        Self {
            is_sync_active: is_tape_broken, // Store original in struct
            editor_state: editor::default_state(),

            gain: FloatParam::new(
                "Gain",
                1.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 10.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(15.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            delay_time_ms: FloatParam::new(
                "Time",
                200.0,
                FloatRange::Linear { min: TIME_MS_MIN, max: TIME_MS_MAX },
            )
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_value_to_string(Arc::new(move |value| {
                    // BUG FIX: Instead of checking the atomic flag, we need to know
                    // if we are in sync mode. But the formatter closure only gives us the 'value'.

                    // To fix this globally, we check the atomic, but we must ensure
                    // the atomic is initialized correctly from the PERSISTED state.
                    if time_sync_flag_for_formatter.load(Ordering::Relaxed) {
                        let normalized = (value - TIME_MS_MIN) / (TIME_MS_MAX - TIME_MS_MIN);
                        let (_, label) = get_beat_info(normalized);
                        label.to_string()
                    } else {
                        format!("{:.1} ms", value)
                    }
                })),

            time_sync: BoolParam::new("Time Sync", sync_default)
                .with_callback(Arc::new(move |value| {
                    time_sync_flag_for_callback.store(value, Ordering::Relaxed);
                })),
            broken_tape: BoolParam::new("Broken", false).with_callback(Arc::new(move |value| {
                // When user clicks button, update the flag!
                tape_broken_flag_for_callback.store(value, Ordering::Relaxed);
            })),
            distortion_mode: BoolParam::new("Tape Only", false).with_callback(Arc::new(
                move |value| {
                    distortion_flag_for_callback.store(value, Ordering::Relaxed);
                },
            )),

            feedback: FloatParam::new("Feedback", 0.3, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit(" %")
                .with_value_to_string(Arc::new(move |value| {
                    if distortion_flag_for_feedback_formatter.load(Ordering::Relaxed) {
                        String::from("0")
                    } else {
                        format!("{:.0}", value * 100.0)
                    }
                }))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new("Mix", 0.3, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit(" %")
                .with_value_to_string(Arc::new(move |value| {
                    if distortion_flag_for_mix_formatter.load(Ordering::Relaxed) {
                        String::from("0")
                    } else {
                        format!("{:.0}", value * 100.0)
                    }
                }))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            ghost_zero: FloatParam::new("😎", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(Arc::new(|_| String::from("0.0"))).hide().hide_in_generic_ui(),

            noise: FloatParam::new(
                "Noise",
                0.8,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(15.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            crackle: FloatParam::new(
                "Crackle",
                0.8,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(15.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            stereo_width: FloatParam::new(
                "Width",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.0,
                },
            )
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

impl Default for TapeDelay {
    fn default() -> Self {

        // 2ms base delay + 1ms max mod depth at 44100 Hz = ~133 samples max
        // Allocate for up to 192kHz: ceil(192000 * 0.003) = 576, keep power-of-two margin
        let corrosion_buf_size = 2048usize;
        Self {
            params: Arc::new(TapeParams::default()),
            delay_buffer_l: Vec::new(),
            delay_buffer_r: Vec::new(),
            write_pos: 0,
            sample_rate: 44100.0,
            current_delay_samples: 0.0,
            // Start LFO at 0
            lfo_phase: 0.0,

            // Filters start "empty" (0.0 energy)
            lp_state_l: 0.0,
            lp_state_r: 0.0,

            // Seed can be any non-zero integer
            rng_seed: 12345,
            dropout_smoother: 1.0, // Start with perfect health
            dropout_timer: 0.0,    // Start with no active dropout

            corrosion_buf_l: vec![0.0; corrosion_buf_size],
            corrosion_buf_r: vec![0.0; corrosion_buf_size],
            corrosion_write: 0,
            corrosion_sine_phase: 0.0,
            corrosion_bp_l: [0.0; 2],
            corrosion_bp_r: [0.0; 2],
            corrosion_rng: 0xDEAD_BEEF,

            meter_decay_per_sample: 1.0,
            peak_meter_l: Arc::new(AtomicF32::new(0.0)), // 0.0 Linear = Silence
            peak_meter_r: Arc::new(AtomicF32::new(0.0)),
            crackle_integrator_l: 0.0,
            crackle_integrator_r: 0.0,
            crackle_hp_l: 0.0,
            crackle_hp_r: 0.0,
            was_distortion_mode: false,
        }
    }
}

impl Plugin for TapeDelay {
    const NAME: &'static str = "Tape Delay";
    const VENDOR: &'static str = "Convolution DEV";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "email@example.com";
    const VERSION: &'static str = "0.1.12";

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        _ctx: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = _buffer_config.sample_rate;
        let max_samples = (self.sample_rate * 2.0) as usize;
        self.delay_buffer_l = vec![0.0; max_samples];
        self.delay_buffer_r = vec![0.0; max_samples];

        // Resize corrosion delay buffers for the actual sample rate.
        // We need at least (base_delay + max_mod_depth) * sample_rate samples:
        //   2ms base + 1ms max depth = 3ms => sample_rate * 0.003, rounded up with margin.
        let corrosion_buf_size =
            ((_buffer_config.sample_rate * 0.004) as usize + 4).next_power_of_two();
        self.corrosion_buf_l = vec![0.0; corrosion_buf_size];
        self.corrosion_buf_r = vec![0.0; corrosion_buf_size];
        self.corrosion_write = 0;

        let release_db_per_second = 30.0;

        // Calculate the constant for 1 sample of decay
        // We store this in the struct
        self.meter_decay_per_sample = f32::powf(
            10.0,
            -release_db_per_second / (20.0 * _buffer_config.sample_rate),
        );

        true
    }

    fn reset(&mut self) {

        // Clear corrosion state
        self.corrosion_buf_l.iter_mut().for_each(|s| *s = 0.0);
        self.corrosion_buf_r.iter_mut().for_each(|s| *s = 0.0);
        self.corrosion_write = 0;
        self.corrosion_sine_phase = 0.0;
        self.corrosion_bp_l = [0.0; 2];
        self.corrosion_bp_r = [0.0; 2];
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let sample_rate = self.sample_rate;
        let buffer_len = self.delay_buffer_l.len();

        // --- STATE MANAGEMENT ---
        let is_distortion_mode = self.params.distortion_mode.value();
        if self.was_distortion_mode && !is_distortion_mode {
            self.delay_buffer_l.fill(0.0);
            self.delay_buffer_r.fill(0.0);
        }
        self.was_distortion_mode = is_distortion_mode;

        let is_broken = self.params.broken_tape.value();
        let tape_constants = calculate_tape_constants(sample_rate, is_broken);
        let width_amt = self.params.stereo_width.value();

        // --- METERING PREP ---
        let mut max_amplitude_in_block_l: f32 = 0.0;
        let mut max_amplitude_in_block_r: f32 = 0.0;

        // --- DELAY TIME CALCULATION (only for delay mode) ---
        let target_delay_samples = if !is_distortion_mode {
            let raw_target_samples = if self.params.time_sync.value() {
                let bpm = _ctx.transport().tempo.unwrap_or(120.0) as f32;
                let seconds_per_beat = 60.0 / bpm;
                let current_ms = self.params.delay_time_ms.value();
                let normalized = (current_ms - TIME_MS_MIN) / (TIME_MS_MAX - TIME_MS_MIN);
                let (multiplier, _) = get_beat_info(normalized);
                (seconds_per_beat * multiplier) * sample_rate
            } else {
                (self.params.delay_time_ms.value() / 1000.0) * sample_rate
            };
            let max_safe_samples = buffer_len as f32 - 100.0;
            raw_target_samples.min(max_safe_samples)
        } else {
            0.0
        };

        // --- MAIN DSP LOOP ---
        for channel_samples in buffer.iter_samples() {
            // --- PER-SAMPLE PARAMETER SMOOTHING ---
            let gain_amt = self.params.gain.smoothed.next();
            let noise_vol = self.params.noise.smoothed.next();
            let crackle_vol = self.params.crackle.smoothed.next();
            let mix_amt = self.params.mix.smoothed.next();
            let feedback_amt = self.params.feedback.smoothed.next();

            // --- GAIN COMPENSATION ---
            let (makeup_gain, compensated_noise_amt, compensated_crackle_amt) =
                calculate_gain_compensation(
                    gain_amt,
                    tape_constants.noise_amount,
                    tape_constants.crackle_amount,
                    noise_vol,
                    crackle_vol,
                );

            // --- DROPOUTS ---
            let vol_mod = update_dropout_smoother(
                is_broken,
                &mut self.dropout_smoother,
                &mut self.dropout_timer,
                &mut self.rng_seed,
                sample_rate,
            );

            // --- NOISE & CRACKLE GENERATION ---
            let (noise_l, crackle_l) = generate_tape_noise_and_crackle(
                &mut self.rng_seed,
                &mut self.crackle_integrator_l,
                &mut self.crackle_hp_l,
                tape_constants.crackle_threshold,
                compensated_noise_amt,
                compensated_crackle_amt,
            );
            let (noise_r, crackle_r) = generate_tape_noise_and_crackle(
                &mut self.rng_seed,
                &mut self.crackle_integrator_r,
                &mut self.crackle_hp_r,
                tape_constants.crackle_threshold,
                compensated_noise_amt,
                compensated_crackle_amt,
            );

            let mut samples = channel_samples.into_iter();
            let sample_l_ref = samples.next().unwrap();
            let sample_r_ref = samples.next().unwrap();
            let input_l = *sample_l_ref;
            let input_r = *sample_r_ref;

            if is_distortion_mode {
                // --- TAPE ONLY / DISTORTION MODE ---
                let mut signal_l = input_l + noise_l + crackle_l;
                let mut signal_r = input_r + noise_r + crackle_r;

                // Apply corrosion if broken
                if is_broken {
                    (signal_l, signal_r) = self.apply_corrosion(sample_rate, signal_l, signal_r);
                }

                signal_l *= vol_mod;
                signal_r *= vol_mod;

                let saturated_l = drive_tape_classic(gain_amt, signal_l);
                let saturated_r = drive_tape_classic(gain_amt, signal_r);

                let filtered_l = one_pole_lp(saturated_l, &mut self.lp_state_l, tape_constants.current_tone_cutoff);
                let filtered_r = one_pole_lp(saturated_r, &mut self.lp_state_r, tape_constants.current_tone_cutoff);

                *sample_l_ref = filtered_l * makeup_gain;
                *sample_r_ref = filtered_r * makeup_gain;

            } else {
                // --- TAPE DELAY MODE ---
                update_lfo_phase(&mut self.lfo_phase, tape_constants.flutter_rate);
                let flutter_depth = 15.0;
                let phase_offset_r = width_amt * std::f32::consts::PI;
                let flutter_offset_l = self.lfo_phase.sin() * flutter_depth;
                let flutter_offset_r = (self.lfo_phase + phase_offset_r).sin() * flutter_depth;

                let smooth_coeff = 0.0005;
                self.current_delay_samples = (self.current_delay_samples * (1.0 - smooth_coeff))
                    + (target_delay_samples * smooth_coeff);

                let spread_samples = width_amt * 0.010 * sample_rate;
                let mod_delay_samples_l = (self.current_delay_samples - spread_samples + flutter_offset_l).max(0.0);
                let mod_delay_samples_r = (self.current_delay_samples + spread_samples + flutter_offset_r).max(0.0);

                let read_pos_l = (self.write_pos as f32 - mod_delay_samples_l).rem_euclid(buffer_len as f32);
                let read_pos_r = (self.write_pos as f32 - mod_delay_samples_r).rem_euclid(buffer_len as f32);

                let raw_delayed_l = linear_interpolate(&self.delay_buffer_l, read_pos_l);
                let raw_delayed_r = linear_interpolate(&self.delay_buffer_r, read_pos_r);

                let feedback_gain = (feedback_amt * 1.2) / gain_amt.sqrt();

                let tone_spread = width_amt * 0.15;
                let cutoff_l = (tape_constants.current_tone_cutoff - tone_spread).max(0.1);
                let cutoff_r = (tape_constants.current_tone_cutoff + tone_spread).min(0.95);

                let filtered_feedback_l = one_pole_lp(raw_delayed_l, &mut self.lp_state_l, cutoff_l);
                let filtered_feedback_r = one_pole_lp(raw_delayed_r, &mut self.lp_state_r, cutoff_r);

                let mut signal_to_record_l = input_l + (filtered_feedback_l * feedback_gain) + noise_l + crackle_l;
                let mut signal_to_record_r = input_r + (filtered_feedback_r * feedback_gain) + noise_r + crackle_r;

                // Apply corrosion if broken
                if is_broken {
                    (signal_to_record_l, signal_to_record_r) = self.apply_corrosion(sample_rate, signal_to_record_l, signal_to_record_r);
                }

                signal_to_record_l *= vol_mod;
                signal_to_record_r *= vol_mod;

                let saturated_l = drive_tape_classic(gain_amt, signal_to_record_l);
                let saturated_r = drive_tape_classic(gain_amt, signal_to_record_r);

                if let Some(buf_val) = self.delay_buffer_l.get_mut(self.write_pos) {
                    *buf_val = saturated_l;
                }
                if let Some(buf_val) = self.delay_buffer_r.get_mut(self.write_pos) {
                    *buf_val = saturated_r;
                }

                let wet_l = raw_delayed_l * makeup_gain;
                let wet_r = raw_delayed_r * makeup_gain;

                *sample_l_ref = (input_l * (1.0 - mix_amt)) + (wet_l * mix_amt);
                *sample_r_ref = (input_r * (1.0 - mix_amt)) + (wet_r * mix_amt);
            }

            // --- METERING ---
            let abs_l = sample_l_ref.abs();
            if abs_l > max_amplitude_in_block_l {
                max_amplitude_in_block_l = abs_l;
            }
            let abs_r = sample_r_ref.abs();
            if abs_r > max_amplitude_in_block_r {
                max_amplitude_in_block_r = abs_r;
            }

            // --- ADVANCE WRITE HEAD (only in delay mode) ---
            if !is_distortion_mode {
                self.write_pos = (self.write_pos + 1) % buffer_len;
            }
        }

        // --- UPDATE METERS (Once per buffer block) ---
        update_peak_meters(
            self.params.editor_state.is_open(),
            buffer.samples() as f32,
            self.meter_decay_per_sample,
            &self.peak_meter_l,
            &self.peak_meter_r,
            max_amplitude_in_block_l,
            max_amplitude_in_block_r,
        );

        ProcessStatus::Normal
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.peak_meter_l.clone(),
            self.peak_meter_r.clone(),
            self.params.editor_state.clone(),
        )
    }
}

impl TapeDelay {

    /// Fractional delay buffer reader using linear interpolation.
    /// `buf` is a circular delay buffer, `write_pos` is the current write head,
    /// `delay_samples` is the number of samples to look back (may be fractional).
    fn corrosion_read(buf: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
        let len = buf.len();
        let delay_i = delay_samples as usize;
        let frac = delay_samples - delay_i as f32;

        // Clamp so we never exceed the buffer length
        let delay_i = delay_i.min(len - 1);

        // Read head (going backward from the write position)
        let idx0 = (write_pos + len - delay_i) % len;
        let idx1 = (write_pos + len - delay_i - 1) % len;

        // Linear interpolation between the two adjacent samples
        buf[idx0] + frac * (buf[idx1] - buf[idx0])
    }

    fn apply_corrosion(&mut self, sample_rate: f32, driven_l: f32, driven_r: f32) -> (f32, f32) {
        let corr_amount = 0.22;
        let (corr_l, corr_r) = if corr_amount > 0.0 {
            let corr_freq: f32 = 911.0;
            let corr_width: f32 = 2.0;
            let corr_blend: f32 = 1.0;
            let corr_stereo: f32 = 0.75;

            // Constants matching the Ableton 12.4 spec
            const BASE_DELAY: f32 = 0.002; // 2 ms
            const MAX_MOD_DEPTH: f32 = 0.001; // 1 ms max fluctuation

            // 1. Sine modulators (stereo phase offset)
            let sine_l = (self.corrosion_sine_phase * std::f32::consts::TAU).sin();
            let sine_r = ((self.corrosion_sine_phase * std::f32::consts::TAU)
                + corr_stereo * std::f32::consts::PI)
                .sin();
            self.corrosion_sine_phase =
                (self.corrosion_sine_phase + corr_freq / sample_rate).fract();

            // 2. Independent white noise for each channel (LCG)
            self.corrosion_rng = self
                .corrosion_rng
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let raw_noise_l = (self.corrosion_rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
            self.corrosion_rng = self
                .corrosion_rng
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let raw_noise_r = (self.corrosion_rng as f32 / u32::MAX as f32) * 2.0 - 1.0;

            // 3. Bandpass-filter noise (2nd-order approximation: LP then HP derived
            //    from LP; bandwidth controlled by corr_width)
            let lp_cutoff = (corr_freq * corr_width.max(0.01)).min(sample_rate * 0.499);
            let hp_cutoff = (corr_freq / corr_width.max(0.01).max(1.0)).max(1.0);
            let dt = 1.0 / sample_rate;
            let lp_a = dt / (1.0 / (std::f32::consts::TAU * lp_cutoff) + dt);
            let hp_a = dt / (1.0 / (std::f32::consts::TAU * hp_cutoff) + dt);

            // Left channel BP
            self.corrosion_bp_l[0] += lp_a * (raw_noise_l - self.corrosion_bp_l[0]);
            let lp_l = self.corrosion_bp_l[0];
            self.corrosion_bp_l[1] += hp_a * (lp_l - self.corrosion_bp_l[1]);
            let bp_l = lp_l - self.corrosion_bp_l[1]; // bandpass = LP - LP-of-LP

            // Right channel BP
            self.corrosion_bp_r[0] += lp_a * (raw_noise_r - self.corrosion_bp_r[0]);
            let lp_r = self.corrosion_bp_r[0];
            self.corrosion_bp_r[1] += hp_a * (lp_r - self.corrosion_bp_r[1]);
            let bp_r = lp_r - self.corrosion_bp_r[1];

            // 4. Stereo decorrelation for noise (lerp from mono L → uncorrelated R)
            let noise_l = bp_l;
            let noise_r = noise_l + corr_stereo * (bp_r - noise_l);

            // 5. Noise-blend: crossfade sine <-> bandpassed noise
            let mod_l = sine_l + corr_blend * (noise_l - sine_l);
            let mod_r = sine_r + corr_blend * (noise_r - sine_r);

            // 6. Convert modulation signal to delay time in samples
            let delay_samples_l =
                (BASE_DELAY + mod_l * corr_amount * MAX_MOD_DEPTH).max(0.0) * sample_rate;
            let delay_samples_r =
                (BASE_DELAY + mod_r * corr_amount * MAX_MOD_DEPTH).max(0.0) * sample_rate;

            // 7. Write input to delay buffers
            let buf_len = self.corrosion_buf_l.len();
            self.corrosion_buf_l[self.corrosion_write] = driven_l;
            self.corrosion_buf_r[self.corrosion_write] = driven_r;

            // 8. Read back with linear interpolation at the modulated delay time
            let read_l = Self::corrosion_read(
                &self.corrosion_buf_l,
                self.corrosion_write,
                delay_samples_l,
            );
            let read_r = Self::corrosion_read(
                &self.corrosion_buf_r,
                self.corrosion_write,
                delay_samples_r,
            );

            self.corrosion_write = (self.corrosion_write + 1) % buf_len;

            (read_l, read_r)
        } else {
            (driven_l, driven_r)
        };
        (corr_l, corr_r)
    }

}

pub fn normalized_to_sync_step(normalized: f32) -> i32 {
    // 1. Multiply by total steps
    let step = normalized * NUM_SYNC_STEPS;

    // 2. Floor to get the index
    let step_i32 = step.floor() as i32;

    // 3. Safety Clamp: Ensure we never go out of bounds (e.g. if normalized is exactly 1.0)
    // The valid range is 0 to 17.
    step_i32.clamp(0, (NUM_SYNC_STEPS as i32) - 1)
}

pub fn get_beat_info(normalized: f32) -> (f32, &'static str) {
    // CALL THE SHARED FUNCTION HERE
    let step_index = normalized_to_sync_step(normalized);

    match step_index {
        0 => (0.0625, "1/64"),   // Straight
        1 => (0.125, "1/32"),    // Straight
        2 => (0.1667, "1/16 T"), // Triplet
        3 => (0.1875, "1/32 ."), // Dotted
        4 => (0.25, "1/16"),
        5 => (0.3333, "1/8 T"),
        6 => (0.375, "1/16 ."),
        7 => (0.5, "1/8"),
        8 => (0.6667, "1/4 T"),
        9 => (0.75, "1/8 ."),
        10 => (1.0, "1/4"),
        11 => (1.3333, "1/2 T"),
        12 => (1.5, "1/4 ."),
        13 => (2.0, "1/2"),
        14 => (2.6667, "1/1 T"), // Whole Triplet
        15 => (3.0, "1/2 ."),
        16 => (4.0, "1 Bar"),
        17 => (8.0, "2 Bar"),
        _ => (1.0, "1/4"), // Should never happen thanks to clamp
    }
}

// Helper: A simple 1-pole lowpass filter (The "Tone Knob")
// value: current sample, state: previous sample, cutoff: 0.0 to 1.0
fn one_pole_lp(input: f32, state: &mut f32, cutoff: f32) -> f32 {
    *state += cutoff * (input - *state);
    *state
}


// Tape Saturation Type 1: Classic Analog Tape (Soft Knee)
// Models vintage tape machines with smooth, musical saturation and hysteresis-like behavior
fn drive_tape_classic(drive: f32, signal: f32) -> f32 {
    let x = signal * drive;

    // Soft saturation curve with tape-like compression
    let saturated = if x.abs() < 0.5 {
        x * (1.0 - 0.15 * x.abs())
    } else {
        let sign = x.signum();
        sign * (0.425 + 0.575 * (1.0 - (-(x.abs() - 0.5) * 3.0).exp()))
    };

    saturated
}

// Helper: Simple pseudo-random noise generator (for Dust/Hiss)
fn get_noise(seed: &mut u32) -> f32 {
    // A quick Linear Congruential Generator (LCG) for speed
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0 // Returns -1.0 to 1.0
}

fn get_crackle(seed: &mut u32, threshold: f32) -> f32 {
    // 1. Roll for Magnitude (Will we crackle?)
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    let random_val = *seed as f32 / u32::MAX as f32;

    if random_val > threshold {
        // 2. Roll again for Polarity (Positive or Negative?)
        // This decouples the "when" from the "which direction"
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);

        // We can check the top bit (MSB) which is the most random part of an LCG
        // 0x80000000 is the binary mask for the first bit
        if (*seed & 0x80000000) != 0 {
            return 0.2; // Positive pop
        } else {
            return -0.2; // Negative pop
        }
    }

    0.0
}

// --- HELPER FUNCTIONS FOR PROCESS LOOP ---

#[inline]
fn calculate_tape_constants(sample_rate: f32, is_broken: bool) -> TapeConstants {
    let flutter_rate = 2.0 * std::f32::consts::PI * (1.5 / sample_rate);
    let noise_amount = 0.005;
    let crackle_amount = 0.15;
    let current_tone_cutoff = if is_broken { 0.45 } else { 0.85 };
    let target_crackle_hz = 3.0;
    let probability_crackle = target_crackle_hz / sample_rate;
    let crackle_threshold = 1.0 - probability_crackle;

    TapeConstants {
        flutter_rate,
        noise_amount,
        crackle_amount,
        current_tone_cutoff,
        crackle_threshold,
    }
}

struct TapeConstants {
    flutter_rate: f32,
    noise_amount: f32,
    crackle_amount: f32,
    current_tone_cutoff: f32,
    crackle_threshold: f32,
}

#[inline]
fn update_lfo_phase(lfo_phase: &mut f32, flutter_rate: f32) {
    *lfo_phase += flutter_rate;
    if *lfo_phase > 2.0 * std::f32::consts::PI {
        *lfo_phase -= 2.0 * std::f32::consts::PI;
    }
}

#[inline]
fn calculate_gain_compensation(
    gain_amt: f32,
    noise_amount: f32,
    crackle_amount: f32,
    noise_volume: f32,
    crackle_volume: f32,
) -> (f32, f32, f32) {
    let makeup_gain = 1.0 / gain_amt.powf(0.60);
    let compensation_factor = gain_amt * makeup_gain;
    let compensated_noise_amt = noise_amount / compensation_factor;
    let compensated_crackle_amt = crackle_amount / compensation_factor;
    let final_noise_amt = compensated_noise_amt * noise_volume;
    let final_crackle_amt = compensated_crackle_amt * crackle_volume;
    (makeup_gain, final_noise_amt, final_crackle_amt)
}

#[inline]
fn update_dropout_smoother(
    is_broken: bool,
    dropout_smoother: &mut f32,
    dropout_timer: &mut f32,
    rng_seed: &mut u32,
    sample_rate: f32,
) -> f32 {
    if is_broken {
        let rand_val = get_noise(rng_seed).abs();

        // 1. CHANCE TO TRIGGER
        // 0.9999 is okay, but if it's too frequent, try 0.99995
        if rand_val > 0.99995 && *dropout_timer <= 0.0 {
            // REDUCED DURATION: Stay at 0.3 for only 5ms to 20ms
            // This is a "micro-dropout"
            *dropout_timer = (0.005 + (rand_val * 0.015)) * sample_rate;
        }

        // 2. DETERMINE TARGET
        let target_health = if *dropout_timer > 0.0 {
            *dropout_timer -= 1.0;
            0.3
        } else {
            1.0
        };

        // 3. REFINED SMOOTHING SPEEDS
        // Base recovery time: 30ms (0.030) - much snappier than 800ms!
        let recovery_speed = 1.0 - f32::exp(-1.0 / (0.030 * sample_rate));

        let coeff = if target_health < *dropout_smoother {
            // ATTACK: 40x faster than recovery (approx 0.7ms)
            // This makes the "dip" feel like a real mechanical glitch
            recovery_speed * 40.0
        } else {
            // RELEASE: The 30ms recovery
            recovery_speed
        };

        *dropout_smoother += (target_health - *dropout_smoother) * coeff;

        // Safety clamp to prevent overshoot
        *dropout_smoother = dropout_smoother.clamp(0.0, 1.0);

        *dropout_smoother
    } else {
        *dropout_smoother = 1.0;
        *dropout_timer = 0.0;
        1.0
    }
}

#[inline]
fn generate_tape_noise_and_crackle(
    rng_seed: &mut u32,
    crackle_integrator: &mut f32, // This is your "Low Frequency" state
    crackle_hp: &mut f32,  // NEW: Adds a "High Pass" state to create a Bandpass
    crackle_threshold: f32,
    compensated_noise_amt: f32,
    compensated_crackle_amt: f32,
) -> (f32, f32) {
    let noise = get_noise(rng_seed) * compensated_noise_amt;
    let crackle_impulse = get_crackle(rng_seed, crackle_threshold);

    // 1. THE BODY (Low Pass)
    // 0.99 = Deep, heavy thud.
    // 0.95 = Lighter tap.
    *crackle_integrator += crackle_impulse;
    *crackle_integrator *= 0.99;

    // 2. THE TIGHTNESS (High Pass / DC Blocker)
    // 0.9 = Tight, punchy kick drum sound.
    // 0.99 = Loose, rumbly sub-bass.
    // 0.8 = Thin, wooden knock.
    let hp_coeff = 0.9;

    // This removes the "infinite rumble" (DC offset) and creates the punch
    let input = *crackle_integrator;
    let output = input - *crackle_hp;
    *crackle_hp = input * (1.0 - hp_coeff) + *crackle_hp * hp_coeff;

    // Apply volume
    let crackle = output * compensated_crackle_amt;
    (noise, crackle)
}

#[inline]
fn update_peak_meters(
    editor_open: bool,
    buffer_samples: f32,
    meter_decay_per_sample: f32,
    peak_meter_l: &Arc<AtomicF32>,
    peak_meter_r: &Arc<AtomicF32>,
    max_amplitude_l: f32,
    max_amplitude_r: f32,
) {
    if !editor_open {
        return;
    }

    let block_decay = f32::powf(meter_decay_per_sample, buffer_samples);

    // Update left meter
    let current_peak_l = peak_meter_l.load(Ordering::Relaxed);
    let mut new_peak_l = if max_amplitude_l > current_peak_l {
        max_amplitude_l
    } else {
        current_peak_l * block_decay
    };
    if new_peak_l < 0.001 {
        new_peak_l = 0.0;
    }
    peak_meter_l.store(new_peak_l, Ordering::Relaxed);

    // Update right meter
    let current_peak_r = peak_meter_r.load(Ordering::Relaxed);
    let mut new_peak_r = if max_amplitude_r > current_peak_r {
        max_amplitude_r
    } else {
        current_peak_r * block_decay
    };
    if new_peak_r < 0.001 {
        new_peak_r = 0.0;
    }
    peak_meter_r.store(new_peak_r, Ordering::Relaxed);
}

impl Vst3Plugin for TapeDelay {
    const VST3_CLASS_ID: [u8; 16] = *b"ConvolutionDelay";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Delay,
        Vst3SubCategory::Modulation,
        Vst3SubCategory::Fx,
    ];
}

#[inline]
fn linear_interpolate(buffer: &[f32], read_pos: f32) -> f32 {
    let len = buffer.len();
    if len == 0 {
        return 0.0;
    }
    if len == 1 {
        return buffer[0];
    }

    // Use floor to get the integer part safely
    let read_pos_floor = read_pos.floor();
    let fraction = read_pos - read_pos_floor;

    // Ensure index_a is within [0, len-1]
    let index_a = (read_pos_floor as usize) % len;
    // Ensure index_b is index_a + 1 wrapped around
    let index_b = (index_a + 1) % len;

    // Use get() to provide a default 0.0 instead of panicking
    // This is the "ultimate" safety net for DSP
    let sample_a = buffer.get(index_a).unwrap_or(&0.0);
    let sample_b = buffer.get(index_b).unwrap_or(&0.0);

    sample_a * (1.0 - fraction) + sample_b * fraction
}

nih_export_vst3!(TapeDelay);
