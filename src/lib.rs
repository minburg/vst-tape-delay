use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::sync::Arc;

mod editor;

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
}

#[derive(Params)]
struct TapeParams {
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,

    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "time"]
    pub delay_time_ms: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}


impl Default for TapeParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),

            gain: FloatParam::new("Gayn", 1.2, FloatRange::Linear { min: 0.5, max: 10.0 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_value_to_string(formatters::v2s_f32_rounded(1)),

            delay_time_ms: FloatParam::new("Tame", 200.0, FloatRange::Linear { min: 1.0, max: 1000.0 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),

            feedback: FloatParam::new("Feedbick", 0.3, FloatRange::Linear { min: 0.0, max: 0.999 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(1))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new("Mics", 0.3, FloatRange::Linear { min: 0.0, max: 1.0 })
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
        }
    }
}

impl Plugin for TapeDelay {
    const NAME: &'static str = "Tape Delay";
    const VENDOR: &'static str = "Convolution DEV";
    const URL: &'static str = "https://youtu.be/dQw4w9WgXcQ";
    const EMAIL: &'static str = "email@example.com";
    const VERSION: &'static str = "0.0.1";

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];

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
        true
    }

    fn reset(&mut self) {
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let sample_rate = self.sample_rate;
        let buffer_len = self.delay_buffer_l.len();

        // 1. Calculate Target Delay
        let target_delay_samples = (self.params.delay_time_ms.value() / 1000.0) * sample_rate;

        // TAPE MECHANICS: Setup constants for the "Machine"
        // In a full plugin, these would be user parameters (e.g., "Age", "Grit")
        let flutter_rate = 2.0 * std::f32::consts::PI * (1.5 / sample_rate); // 1.5 Hz wobble
        let flutter_depth = 15.0; // The depth of the pitch wobble in samples
        // let tape_drive = 1.2;     // Pushing the "tape" slightly into the red
        let noise_amount = 0.01; // Background hiss level (was 0.002)
        let tone_cutoff = 0.5;    // Low pass filter coefficient (simulates head degradation)

        if buffer_len == 0 { return ProcessStatus::Normal; }

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

            // Add the flutter to the smoothed delay time
            let mod_delay_samples = self.current_delay_samples + flutter_offset;

            // Get Parameter Values
            let feedback_amt = self.params.feedback.smoothed.next();
            let mix_amt = self.params.mix.smoothed.next();

            // --- READ HEAD CALCULATION ---
            let read_pos = (self.write_pos as f32 - mod_delay_samples).rem_euclid(buffer_len as f32);

            let mut samples = channel_samples.into_iter();

            // --- LEFT CHANNEL PROCESSING ---
            if let Some(sample_l) = samples.next() {
                let input_l = *sample_l;

                // 1. Read from "Tape"
                let raw_delayed_l = linear_interpolate(&self.delay_buffer_l, read_pos);

                // 2. Generate Dust/Noise
                let noise = get_noise(&mut self.rng_seed) * noise_amount;

                // 3. Feedback Processing Chain (The "Secret Sauce")
                // We take the delayed signal and process it BEFORE putting it back in the buffer.

                // A. Tone Loss (Filtering)
                // Simulates the high-frequency loss of magnetic tape
                let filtered_feedback = one_pole_lp(raw_delayed_l, &mut self.lp_state_l, tone_cutoff);

                // B. Summing
                // Add input + filtered feedback + a little noise
                let mut signal_to_record = input_l + (filtered_feedback * feedback_amt) + noise;

                // C. Tape Saturation (Soft Clipping)
                // This is CRITICAL. It squashes the signal.
                // If feedback > 100%, this keeps it from exceeding digital max (1.0).
                signal_to_record = soft_clip(signal_to_record, self.params.feedback.smoothed.next());

                // 4. Write to "Tape" (Buffer)
                if let Some(buf_val) = self.delay_buffer_l.get_mut(self.write_pos) {
                    *buf_val = signal_to_record;
                }

                // 5. Output Mix
                // Usually on tape delays, the output is also saturated, or you blend the clean dry with saturated wet.
                *sample_l = (input_l * (1.0 - mix_amt)) + (raw_delayed_l * mix_amt);
            }

            // --- RIGHT CHANNEL PROCESSING (Same logic) ---
            if let Some(sample_r) = samples.next() {
                let input_r = *sample_r;
                let raw_delayed_r = linear_interpolate(&self.delay_buffer_r, read_pos);
                let noise = get_noise(&mut self.rng_seed) * noise_amount;

                let filtered_feedback = one_pole_lp(raw_delayed_r, &mut self.lp_state_r, tone_cutoff);
                let mut signal_to_record = input_r + (filtered_feedback * feedback_amt) + noise;

                signal_to_record = soft_clip(signal_to_record, self.params.feedback.smoothed.next());

                if let Some(buf_val) = self.delay_buffer_r.get_mut(self.write_pos) {
                    *buf_val = signal_to_record;
                }

                *sample_r = (input_r * (1.0 - mix_amt)) + (raw_delayed_r * mix_amt);
            }

            // Increment Write Head
            self.write_pos = (self.write_pos + 1) % buffer_len;
        }

        ProcessStatus::Normal
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.params.editor_state.clone(),
        )
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

impl Vst3Plugin for TapeDelay {
    const VST3_CLASS_ID: [u8; 16] = *b"TapeDelayPlug123";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Delay, Vst3SubCategory::Modulation, Vst3SubCategory::Fx];
}

#[inline]
fn linear_interpolate(buffer: &[f32], read_pos: f32) -> f32 {
    let len = buffer.len();
    if len == 0 { return 0.0; }
    if len == 1 { return buffer[0]; }

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
