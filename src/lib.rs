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
    const VERSION: &'static str = "0.1.7";

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

        let release_db_per_second = 30.0;

        // Calculate the constant for 1 sample of decay
        // We store this in the struct
        self.meter_decay_per_sample = f32::powf(
            10.0,
            -release_db_per_second / (20.0 * _buffer_config.sample_rate),
        );

        true
    }

    fn reset(&mut self) {}

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let sample_rate = self.sample_rate;
        let buffer_len = self.delay_buffer_l.len();

        // 1. Get Current State
        let is_distortion_mode = self.params.distortion_mode.value();

        // 2. EDGE DETECTION: Check if we just switched OFF distortion mode
        if self.was_distortion_mode && !is_distortion_mode {
            // Clear the buffers to remove old "ghost" echoes
            self.delay_buffer_l.fill(0.0);
            self.delay_buffer_r.fill(0.0);

            // Optional: Reset write head to avoid clicking (optional, but cleaner)
            // self.write_pos = 0;
        }

        // 3. Update the state tracker for the NEXT block
        self.was_distortion_mode = is_distortion_mode;

        // --- DISTORTION MODE: Direct Signal Path ---
        if is_distortion_mode {
            let is_broken = self.params.broken_tape.value();
            let tape_constants = calculate_tape_constants(sample_rate, is_broken);

            let mut max_amplitude_in_block_l: f32 = 0.0;
            let mut max_amplitude_in_block_r: f32 = 0.0;

            for channel_samples in buffer.iter_samples() {
                update_lfo_phase(&mut self.lfo_phase, tape_constants.flutter_rate);

                let gain_amt = self.params.gain.smoothed.next();
                let noise_amt = self.params.noise.smoothed.next();
                let crackle_amt = self.params.crackle.smoothed.next();
                let (makeup_gain, compensated_noise_amt, compensated_crackle_amt ) =
                    calculate_gain_compensation(
                        gain_amt,
                        tape_constants.noise_amount,
                        tape_constants.crackle_amount,
                        noise_amt,
                        crackle_amt,
                    );

                let vol_mod = update_dropout_smoother(
                    is_broken,
                    &mut self.dropout_smoother,
                    &mut self.dropout_timer,
                    &mut self.rng_seed,
                    sample_rate,
                );

                let mut samples = channel_samples.into_iter();

                // --- LEFT CHANNEL DIRECT PROCESSING ---
                if let Some(sample_l) = samples.next() {
                    *sample_l = process_direct_distortion_channel(
                        *sample_l,
                        &mut self.lp_state_l,
                        &mut self.rng_seed,
                        &mut self.crackle_integrator_l,
                        &mut self.crackle_hp_l,
                        tape_constants.current_tone_cutoff,
                        tape_constants.crackle_threshold,
                        compensated_noise_amt,
                        compensated_crackle_amt,
                        vol_mod,
                        gain_amt,
                        makeup_gain,
                    );

                    let abs_l = sample_l.abs();
                    if abs_l > max_amplitude_in_block_l {
                        max_amplitude_in_block_l = abs_l;
                    }
                }

                // --- RIGHT CHANNEL DIRECT PROCESSING ---
                if let Some(sample_r) = samples.next() {
                    *sample_r = process_direct_distortion_channel(
                        *sample_r,
                        &mut self.lp_state_r,
                        &mut self.rng_seed,
                        &mut self.crackle_integrator_r,
                        &mut self.crackle_hp_r,
                        tape_constants.current_tone_cutoff,
                        tape_constants.crackle_threshold,
                        compensated_noise_amt,
                        compensated_crackle_amt,
                        vol_mod,
                        gain_amt,
                        makeup_gain,
                    );

                    let abs_r = sample_r.abs();
                    if abs_r > max_amplitude_in_block_r {
                        max_amplitude_in_block_r = abs_r;
                    }
                }
            }

            update_peak_meters(
                self.params.editor_state.is_open(),
                buffer.samples() as f32,
                self.meter_decay_per_sample,
                &self.peak_meter_l,
                &self.peak_meter_r,
                max_amplitude_in_block_l,
                max_amplitude_in_block_r,
            );

            return ProcessStatus::Normal;
        }

        // --- TAPE DELAY MODE (Original Code) ---
        // 1. DETERMINE TARGET DELAY (Dual Mode Logic)
        let raw_target_samples = if self.params.time_sync.value() {
            // --- SYNC MODE (Stepped) ---

            // A. Get BPM (Default to 120.0)
            let bpm = _ctx.transport().tempo.unwrap_or(120.0) as f32;
            let seconds_per_beat = 60.0 / bpm;

            // B. Map the Knob to 16 Steps
            // We need the "normalized" value of the knob (0.0 to 1.0).
            // If your framework provides .normalized_value(), use that.
            // If not, we calculate it manually assuming range is 1.0 to 1000.0 ms.
            let current_ms = self.params.delay_time_ms.value();

            // Normalize: (Val - Min) / (Max - Min) -> Result is 0.0 to 1.0
            let normalized = (current_ms - TIME_MS_MIN) / (TIME_MS_MAX - TIME_MS_MIN);

            // C. Get Multiplier
            let (multiplier, _) = get_beat_info(normalized);

            // D. Calculate Samples
            (seconds_per_beat * multiplier) * sample_rate
        } else {
            // --- FREE MODE (Continuous) ---
            (self.params.delay_time_ms.value() / 1000.0) * sample_rate
        };

        // 2. SAFETY CLAMP (The "Safe Target")
        // Ensure we never try to read past the end of the allocated buffer.
        // We leave a 100-sample margin for the interpolation and jitter.
        let max_safe_samples = buffer_len as f32 - 100.0;

        // The .min() function returns the smaller of the two values.
        // If raw_target is 350,000 but buffer is only 100,000, this snaps it to 99,900.
        let target_delay_samples = raw_target_samples.min(max_safe_samples);

        let is_broken = self.params.broken_tape.value();
        let tape_constants = calculate_tape_constants(sample_rate, is_broken);
        let flutter_depth = 15.0;

        // --- STEREOIZER: GET KNOB VALUE ---
        // Range 0.0 (Mono) to 1.0 (Super Wide)
        let width_amt = self.params.stereo_width.value();

        if buffer_len == 0 {
            return ProcessStatus::Normal;
        }

        // --- METERING PREP ---
        // We want to find the loudest peak in this entire buffer block
        let mut max_amplitude_in_block_l: f32 = 0.0;
        let mut max_amplitude_in_block_r: f32 = 0.0;

        for channel_samples in buffer.iter_samples() {
            update_lfo_phase(&mut self.lfo_phase, tape_constants.flutter_rate);

            // --- A. STEREO WOBBLE (LFO Decorrelation) ---
            // Left channel uses normal LFO phase.
            // Right channel gets phase-shifted based on width.
            // At width 1.0, offset is PI (180 deg), meaning L pitches UP while R pitches DOWN.
            let phase_offset_r = width_amt * std::f32::consts::PI;

            let flutter_offset_l = self.lfo_phase.sin() * flutter_depth;
            let flutter_offset_r = (self.lfo_phase + phase_offset_r).sin() * flutter_depth;

            // Smooth delay time (Slew Limiting)
            let smooth_coeff = 0.0005;
            self.current_delay_samples = (self.current_delay_samples * (1.0 - smooth_coeff))
                + (target_delay_samples * smooth_coeff);

            // --- B. STEREO SKEW (Haas Offset) ---
            // We push the heads apart by up to 10ms (approx 441 samples at 44.1k).
            // L reads earlier (-), R reads later (+).
            let spread_samples = width_amt * 0.010 * sample_rate;

            // Calculate independent delay times for L and R
            let mod_delay_samples_l = (self.current_delay_samples - spread_samples + flutter_offset_l).max(0.0);
            let mod_delay_samples_r = (self.current_delay_samples + spread_samples + flutter_offset_r).max(0.0);

            let mix_amt = if is_distortion_mode {
                0.0
            } else {
                self.params.mix.smoothed.next()
            };
            let gain_amt = self.params.gain.smoothed.next();
            let noise_amt = self.params.noise.smoothed.next();
            let crackle_amt = self.params.crackle.smoothed.next();
            let feedback_gain = if is_distortion_mode {
                0.0
            } else {
                (self.params.feedback.smoothed.next() * 1.2) / gain_amt.sqrt()
            };

            let (makeup_gain, compensated_noise_amt, compensated_crackle_amt) =
                calculate_gain_compensation(
                    gain_amt,
                    tape_constants.noise_amount,
                    tape_constants.crackle_amount,
                    noise_amt,
                    crackle_amt,
                );

            let vol_mod = update_dropout_smoother(
                is_broken,
                &mut self.dropout_smoother,
                &mut self.dropout_timer,
                &mut self.rng_seed,
                sample_rate,
            );

            // --- C. STEREO TONE (Psychoacoustic Separation) ---
            // As width increases, spread the filter cutoffs.
            // L gets darker, R gets brighter.
            let tone_spread = width_amt * 0.15;
            let cutoff_l = (tape_constants.current_tone_cutoff - tone_spread).max(0.1);
            let cutoff_r = (tape_constants.current_tone_cutoff + tone_spread).min(0.95);

            // --- READ HEAD CALCULATION (Split L/R) ---
            let read_pos_l = (self.write_pos as f32 - mod_delay_samples_l).rem_euclid(buffer_len as f32);
            let read_pos_r = (self.write_pos as f32 - mod_delay_samples_r).rem_euclid(buffer_len as f32);

            let mut samples = channel_samples.into_iter();

            // --- LEFT CHANNEL PROCESSING ---
            if let Some(sample_l) = samples.next() {
                let input_l = *sample_l;
                let raw_delayed_l = linear_interpolate(&self.delay_buffer_l, read_pos_l);

                let (signal_to_record, output) = process_delay_channel(
                    input_l,
                    raw_delayed_l,
                    &mut self.lp_state_l,
                    &mut self.rng_seed,
                    &mut self.crackle_integrator_l,
                    &mut self.crackle_hp_l,
                    cutoff_l,
                    tape_constants.crackle_threshold,
                    compensated_noise_amt,
                    compensated_crackle_amt,
                    feedback_gain,
                    vol_mod,
                    gain_amt,
                    makeup_gain,
                    mix_amt,
                );

                if let Some(buf_val) = self.delay_buffer_l.get_mut(self.write_pos) {
                    *buf_val = signal_to_record;
                }

                *sample_l = output;

                let abs_l = sample_l.abs();
                if abs_l > max_amplitude_in_block_l {
                    max_amplitude_in_block_l = abs_l;
                }
            }

            // --- RIGHT CHANNEL PROCESSING ---
            if let Some(sample_r) = samples.next() {
                let input_r = *sample_r;
                let raw_delayed_r = linear_interpolate(&self.delay_buffer_r, read_pos_r);

                let (signal_to_record, output) = process_delay_channel(
                    input_r,
                    raw_delayed_r,
                    &mut self.lp_state_r,
                    &mut self.rng_seed,
                    &mut self.crackle_integrator_r,
                    &mut self.crackle_hp_r,
                    cutoff_r,
                    tape_constants.crackle_threshold,
                    compensated_noise_amt,
                    compensated_crackle_amt,
                    feedback_gain,
                    vol_mod,
                    gain_amt,
                    makeup_gain,
                    mix_amt,
                );

                if let Some(buf_val) = self.delay_buffer_r.get_mut(self.write_pos) {
                    *buf_val = signal_to_record;
                }

                *sample_r = output;

                let abs_r = sample_r.abs();
                if abs_r > max_amplitude_in_block_r {
                    max_amplitude_in_block_r = abs_r;
                }
            }

            // Increment Write Head
            self.write_pos = (self.write_pos + 1) % buffer_len;
        }

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

// Helper: A simple soft clipper using tanh (The "Tape Saturation")
// This keeps your feedback from exploding.
fn soft_clip(sample: f32, drive: f32) -> f32 {
    // 'drive' allows us to push into the saturation harder
    (sample * drive).tanh()
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
fn process_direct_distortion_channel(
    input: f32,
    lp_state: &mut f32,
    rng_seed: &mut u32,
    crackle_integrator: &mut f32,
    crackle_hp: &mut f32,
    tone_cutoff: f32,
    crackle_threshold: f32,
    compensated_noise_amt: f32,
    compensated_crackle_amt: f32,
    vol_mod: f32,
    gain_amt: f32,
    makeup_gain: f32,
) -> f32 {
    let (noise, crackle) = generate_tape_noise_and_crackle(
        rng_seed,
        crackle_integrator,
        crackle_hp,
        crackle_threshold,
        compensated_noise_amt,
        compensated_crackle_amt,
    );

    let mut signal = input + noise + crackle;
    signal *= vol_mod;
    signal = soft_clip(signal, gain_amt);

    let filtered_output = one_pole_lp(signal, lp_state, tone_cutoff);
    filtered_output * makeup_gain
}

#[inline]
fn process_delay_channel(
    input: f32,
    raw_delayed: f32,
    lp_state: &mut f32,
    rng_seed: &mut u32,
    crackle_integrator: &mut f32,
    crackle_hp: &mut f32,
    tone_cutoff: f32,
    crackle_threshold: f32,
    compensated_noise_amt: f32,
    compensated_crackle_amt: f32,
    feedback_gain: f32,
    vol_mod: f32,
    gain_amt: f32,
    makeup_gain: f32,
    mix_amt: f32,
) -> (f32, f32) {
    let (noise, crackle) = generate_tape_noise_and_crackle(
        rng_seed,
        crackle_integrator,
        crackle_hp,
        crackle_threshold,
        compensated_noise_amt,
        compensated_crackle_amt,
    );

    let filtered_feedback = one_pole_lp(raw_delayed, lp_state, tone_cutoff);
    let mut signal_to_record = input + (filtered_feedback * feedback_gain) + noise + crackle;
    signal_to_record *= vol_mod;
    signal_to_record = soft_clip(signal_to_record, gain_amt);

    let wet_signal = raw_delayed * makeup_gain;
    let output = (input * (1.0 - mix_amt)) + (wet_signal * mix_amt);

    (signal_to_record, output)
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
