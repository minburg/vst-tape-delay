use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod editor;

const TIME_MS_MIN: f32 = 1.0;
const TIME_MS_MAX: f32 = 1500.0;

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

    /// The decay factor for a single sample
    meter_decay_per_sample: f32,
    peak_meter_l: Arc<AtomicF32>,
    peak_meter_r: Arc<AtomicF32>,
}

#[derive(Params)]
struct TapeParams {
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,

    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "time"]
    pub delay_time_ms: FloatParam,
    // Shared flag: Formatter reads this, Sync param updates this.
    // We skip serializing this because it's just a helper for the GUI text.
    #[persist = "ignore"]
    pub is_sync_active: Arc<AtomicBool>,
    #[id = "time_sync"]
    pub time_sync: BoolParam,
    #[id = "broken_tape"]
    pub broken_tape: BoolParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for TapeParams {
    fn default() -> Self {
        // Create the shared memory flag
        let is_time_sync_active = Arc::new(AtomicBool::new(false));

        // Clone it for the closure
        let time_sync_flag_for_formatter = is_time_sync_active.clone();
        let time_sync_flag_for_callback = is_time_sync_active.clone();

        // Create the shared memory flag
        let is_tape_broken = Arc::new(AtomicBool::new(false));

        // Clone it for the closure
        let tape_broken_flag_for_callback = is_tape_broken.clone();

        Self {
            is_sync_active: is_tape_broken, // Store original in struct
            editor_state: editor::default_state(),

            gain: FloatParam::new(
                "Gaen",
                1.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 10.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(15.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            delay_time_ms: FloatParam::new(
                "Tame",
                200.0,
                FloatRange::Linear {
                    min: TIME_MS_MIN,
                    max: TIME_MS_MAX,
                },
            )
            .with_smoother(SmoothingStyle::Linear(15.0))
            .with_value_to_string(Arc::new(move |value| {
                // Check the flag
                if time_sync_flag_for_formatter.load(Ordering::Relaxed) {
                    // --- SYNC MODE DISPLAY ---
                    // Map 1.0-1000.0 back to 0-15 index
                    let normalized = (value - TIME_MS_MIN) / (TIME_MS_MAX - TIME_MS_MIN);
                    let step_index = (normalized * 17.99).floor() as i32;

                    // Get the label (e.g., "1/8 .")
                    let (_, label) = get_beat_info(step_index);
                    label.to_string()
                } else {
                    // --- FREE MODE DISPLAY ---
                    format!("{:.1} ms", value)
                }
            })),

            time_sync: BoolParam::new("Time Sync", false).with_callback(Arc::new(move |value| {
                // When user clicks button, update the flag!
                time_sync_flag_for_callback.store(value, Ordering::Relaxed);
            })),
            broken_tape: BoolParam::new("Broken", false).with_callback(Arc::new(move |value| {
                // When user clicks button, update the flag!
                tape_broken_flag_for_callback.store(value, Ordering::Relaxed);
            })),

            feedback: FloatParam::new(
                "Feed-bick",
                0.3,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.999,
                },
            )
            .with_smoother(SmoothingStyle::Linear(15.0))
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(1))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new("Mic's", 0.3, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(1))
                .with_string_to_value(formatters::s2v_f32_percentage()),
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
            meter_decay_per_sample: 1.0,
            peak_meter_l: Arc::new(AtomicF32::new(0.0)), // 0.0 Linear = Silence
            peak_meter_r: Arc::new(AtomicF32::new(0.0)),
        }
    }
}

impl Plugin for TapeDelay {
    const NAME: &'static str = "Tape Delay";
    const VENDOR: &'static str = "Convolution DEV";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "email@example.com";
    const VERSION: &'static str = "0.0.1";

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

        let release_db_per_second = 20.0;

        // Calculate the constant for 1 sample of decay
        // We store this in the struct
        self.meter_decay_per_sample = f32::powf(
            10.0,
            -release_db_per_second / (20.0 * _buffer_config.sample_rate)
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

            // Map 0.0-1.0 to 0-15 (16 steps)
            // We use .floor() to create stable "zones" for each step.
            let total_steps = 16.0;
            let step_index = (normalized * (total_steps - 0.01)).floor() as i32;

            // C. Get Multiplier
            let multiplier = get_beat_info(step_index).0; // Uses the helper function from before

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

        // TAPE MECHANICS: Setup constants for the "Machine"
        // In a full plugin, these would be user parameters (e.g., "Age", "Grit")
        let flutter_rate = 2.0 * std::f32::consts::PI * (1.5 / sample_rate); // 1.5 Hz wobble
        let flutter_depth = 15.0; // The depth of the pitch wobble in samples
        let noise_amount = 0.005; // Background hiss level (was 0.002)
        let crackle_amount = 0.15; // Background crackle level
        let is_broken = self.params.broken_tape.value();
        // adjust constants based on mode
        // Wasted tape is much darker (lower cutoff)
        let current_tone_cutoff = if is_broken { 0.25 } else { 0.5 };

        // --- CALCULATE CRACKLE THRESHOLD ---
        // How many pops per second do we want?
        // 3.0 Hz = 3 pops per second (approx one every 333ms)
        let target_crackle_hz = 3.0;
        // Calculate probability: 3.0 / 44100.0 = 0.000068...
        let probability_crackle = target_crackle_hz / sample_rate;
        // The random generator produces 0.0 to 1.0.
        // We want the top 0.000068% of values.
        let crackle_threshold = 1.0 - probability_crackle;

        if buffer_len == 0 {
            return ProcessStatus::Normal;
        }

        // --- METERING PREP ---
        // We want to find the loudest peak in this entire buffer block
        let mut max_amplitude_in_block_l: f32 = 0.0;
        let mut max_amplitude_in_block_r: f32 = 0.0;

        for channel_samples in buffer.iter_samples() {
            // --- MODULATION (Wow/Flutter) ---
            // Increment LFO phase. "sin" creates the smooth motor wobble.
            self.lfo_phase += flutter_rate;
            if self.lfo_phase > 2.0 * std::f32::consts::PI {
                self.lfo_phase -= 2.0 * std::f32::consts::PI;
            }
            let flutter_offset = self.lfo_phase.sin() * flutter_depth;

            // Smooth delay time (Slew Limiting)
            // This prevents zipper noise when you turn the time knob quickly.
            let smooth_coeff = 0.0005;
            self.current_delay_samples = (self.current_delay_samples * (1.0 - smooth_coeff))
                + (target_delay_samples * smooth_coeff);

            // FIX: Ensure delay never goes negative (Causality Check)
            // If delay is 0ms, flutter might make it -15. We clamp to 0.0.
            let mod_delay_samples = (self.current_delay_samples + flutter_offset).max(0.0);

            // Get Parameter Values
            let mix_amt = self.params.mix.smoothed.next();
            let gain_amt = self.params.gain.smoothed.next();

            let feedback_gain = (self.params.feedback.smoothed.next() * 1.2) / gain_amt.sqrt();
            let makeup_gain = 1.0 / gain_amt.powf(0.35);

            // 3. NOISE COMPENSATION
            // We divide by (gain * makeup) to ensure the final output volume of the noise
            // stays constant, regardless of how much we drive or attenuate the signal.
            // Logic: Input / (G*M) * G * M = Input.
            let compensation_factor = gain_amt * makeup_gain;
            let compensated_noise_amt = noise_amount / compensation_factor;
            let compensated_crackle_amt = crackle_amount / compensation_factor;
            // B. Feedback Compensation
            // We want High Drive to create specific "gritty" textures, but not instant self-oscillation.
            // We scale feedback down as drive goes up.
            // Using .sqrt() is a musical sweet spot: Higher drive will still feedback MORE than
            // clean tape (exciting!), but not 10x more.
            // Allow feedback to go > 100%.
            // If param is 1.0, internal feedback is 1.2.
            // This ensures the loop gets louder than the input.

            // --- 1. CALCULATE DROPOUTS (The new mechanics) ---
            let mut vol_mod = 1.0;

            if is_broken {
                // A. Generate "Bad Spots"
                // We use a high threshold on the RNG.
                // 0.9997 means a dropout happens roughly every 3000-4000 samples (~0.1 sec)
                let rand_val = get_noise(&mut self.rng_seed).abs(); // 0.0 to 1.0

                let target_health = if rand_val > 0.9995 {
                    0.1 // Deep dropout (almost silent)
                } else if rand_val > 0.99 {
                    0.7 // Minor fluctuation
                } else {
                    1.0 // Healthy tape
                };

                // B. Smooth the dropout (Slew Limiting)
                // This creates the "fading" effect of a dropout rather than a click.
                // 0.005 is the reaction speed.
                self.dropout_smoother += (target_health - self.dropout_smoother) * 0.005;

                vol_mod = self.dropout_smoother;
            } else {
                // Reset to healthy if mode is off
                self.dropout_smoother = 1.0;
            }


            // --- READ HEAD CALCULATION ---
            let read_pos =
                (self.write_pos as f32 - mod_delay_samples).rem_euclid(buffer_len as f32);

            let mut samples = channel_samples.into_iter();

            // --- LEFT CHANNEL PROCESSING ---
            if let Some(sample_l) = samples.next() {
                let input_l = *sample_l;

                // 1. Read from "Tape"
                let raw_delayed_l = linear_interpolate(&self.delay_buffer_l, read_pos);

                // 2. Generate Dust/Noise
                let noise = get_noise(&mut self.rng_seed) * compensated_noise_amt;
                let crackle = get_crackle(&mut self.rng_seed, crackle_threshold) * compensated_crackle_amt;

                // 3. Feedback Processing Chain (The "Secret Sauce")
                // We take the delayed signal and process it BEFORE putting it back in the buffer.

                // A. Tone Loss (Filtering)
                // Simulates the high-frequency loss of magnetic tape
                // Use current_tone_cutoff which changes based on mode
                let filtered_feedback = one_pole_lp(raw_delayed_l, &mut self.lp_state_l, current_tone_cutoff);

                // B. Summing
                // Add input + filtered feedback + a little noise
                let mut signal_to_record =
                    input_l + (filtered_feedback * feedback_gain) + noise + crackle;

                // --- APPLY WASTED EFFECTS ---
                // We apply the dropout *before* saturation.
                // This mimics the tape head losing contact with the magnetic medium.
                // It effectively lowers the drive momentarily, cleaning up the sound while quieting it.
                signal_to_record *= vol_mod;

                // C. Tape Saturation (Soft Clipping)
                // This is CRITICAL. It squashes the signal.
                // If feedback > 100%, this keeps it from exceeding digital max (1.0).
                signal_to_record = soft_clip(signal_to_record, gain_amt);

                // 4. Write to "Tape" (Buffer)
                if let Some(buf_val) = self.delay_buffer_l.get_mut(self.write_pos) {
                    *buf_val = signal_to_record;
                }

                // We apply makeup_gain ONLY to the wet signal here.
                let wet_signal = raw_delayed_l * makeup_gain;

                // 5. Output Mix
                // Usually on tape delays, the output is also saturated, or you blend the clean dry with saturated wet.
                *sample_l = (input_l * (1.0 - mix_amt)) + (wet_signal * mix_amt);

                // Capture Peak for Metering (Simple absolute value check)
                let abs_l = sample_l.abs();
                if abs_l > max_amplitude_in_block_l {
                    max_amplitude_in_block_l = abs_l;
                }
            }

            // --- RIGHT CHANNEL PROCESSING (Same logic) ---
            if let Some(sample_r) = samples.next() {
                let input_r = *sample_r;
                let raw_delayed_r = linear_interpolate(&self.delay_buffer_r, read_pos);
                let noise = get_noise(&mut self.rng_seed) * compensated_noise_amt;
                let crackle = get_crackle(&mut self.rng_seed, crackle_threshold) * compensated_crackle_amt;

                let filtered_feedback = one_pole_lp(raw_delayed_r, &mut self.lp_state_r, current_tone_cutoff);
                let mut signal_to_record =
                    input_r + (filtered_feedback * feedback_gain) + noise + crackle;

                signal_to_record *= vol_mod;
                signal_to_record = soft_clip(signal_to_record, gain_amt);

                if let Some(buf_val) = self.delay_buffer_r.get_mut(self.write_pos) {
                    *buf_val = signal_to_record;
                }

                let wet_signal = raw_delayed_r * makeup_gain;

                *sample_r = (input_r * (1.0 - mix_amt)) + (wet_signal * mix_amt);

                // Capture Peak for Metering (Simple absolute value check)
                let abs_r = sample_r.abs();
                if abs_r > max_amplitude_in_block_r {
                    max_amplitude_in_block_r = abs_r;
                }
            }

            // Increment Write Head
            self.write_pos = (self.write_pos + 1) % buffer_len;
        }

        // --- UPDATE METER (Once per buffer, efficient and thread-safe) ---
        if self.params.editor_state.is_open() {
            // Calculate the decay for THIS specific block size.
            // This makes the decay speed identical whether the buffer is 32 or 1024.
            let block_size = buffer.samples() as f32;
            let block_decay = f32::powf(self.meter_decay_per_sample, block_size);

            // LEFT Meter
            let current_peak_l = self.peak_meter_l.load(std::sync::atomic::Ordering::Relaxed);
            let mut new_peak_l = if max_amplitude_in_block_l > current_peak_l {
                max_amplitude_in_block_l // Attack is instant
            } else {
                // Decay
                current_peak_l * block_decay
            };
            // SAFETY: If the value is tiny, just kill it so the GUI goes to 0
            if new_peak_l < 0.001 {
                new_peak_l = 0.0;
            }
            self.peak_meter_l
                .store(new_peak_l, std::sync::atomic::Ordering::Relaxed);

            // RIGHT Meter
            let current_peak_r = self.peak_meter_r.load(std::sync::atomic::Ordering::Relaxed);
            let mut new_peak_r = if max_amplitude_in_block_r > current_peak_r {
                max_amplitude_in_block_r // Attack is instant
            } else {
                // Decay
                current_peak_r * block_decay

            };
            // SAFETY: If the value is tiny, just kill it so the GUI goes to 0
            if new_peak_r < 0.001 {
                new_peak_r = 0.0;
            }
            self.peak_meter_r
                .store(new_peak_r, std::sync::atomic::Ordering::Relaxed);
        }

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

// Returns (Multiplier, Label)
// Ordered by musical length (Shortest -> Longest)
fn get_beat_info(step_index: i32) -> (f32, &'static str) {
    match step_index {
        0 => (0.0625, "1/64"),    // Straight
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
        _ => (1.0, "1/4"), // Fallback
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
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    let random_val = *seed as f32 / u32::MAX as f32;

    if random_val > threshold {
        // Bipolar pop
        if (*seed & 1) == 0 {
            return 0.2;
        } else {
            return -0.2;
        }
    }
    0.0
}

impl Vst3Plugin for TapeDelay {
    const VST3_CLASS_ID: [u8; 16] = *b"TapeDelayPlug123";
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
