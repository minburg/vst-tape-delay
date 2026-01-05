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
}

#[derive(Params)]
struct TapeParams {
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,

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

            delay_time_ms: FloatParam::new("Time", 15.0, FloatRange::Linear { min: 0.1, max: 50.0 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit("ms")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),

            feedback: FloatParam::new("Feedback", 0.0, FloatRange::Linear { min: 0.0, max: 0.999 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(1))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new("Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0))
                .with_unit("%")
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
        let target_delay_samples = (self.params.delay_time_ms.value() / 1000.0) * sample_rate;

        // Smooth coefficients should ideally be pre-calculated or sample-rate dependent
        let smooth_coeff = 0.0005;
        let smooth_anti_coeff = 1.0 - smooth_coeff;

        let buffer_len = self.delay_buffer_l.len();

        // 1. Safety Guard: Prevent division by zero or processing on empty buffers
        if buffer_len == 0 {
            return ProcessStatus::Normal;
        }

        for channel_samples in buffer.iter_samples() {
            // 2. Smooth parameters once per sample
            self.current_delay_samples = (smooth_anti_coeff * self.current_delay_samples)
                + (smooth_coeff * target_delay_samples);

            let feedback = self.params.feedback.smoothed.next();
            let mix = self.params.mix.smoothed.next();

            // 3. Deterministic read position calculation
            // rem_euclid handles negative wrap-around in one step
            let read_pos = (self.write_pos as f32 - self.current_delay_samples).rem_euclid(buffer_len as f32);

            // 4. Safe Channel Iteration (No Unwraps)
            let mut samples = channel_samples.into_iter();

            // Channel Left
            if let Some(sample_l) = samples.next() {
                let input_l = *sample_l;
                let delayed_l = linear_interpolate(&self.delay_buffer_l, read_pos);

                // Safe indexing
                if let Some(buf_val) = self.delay_buffer_l.get_mut(self.write_pos) {
                    *buf_val = input_l + (delayed_l * feedback);
                }
                *sample_l = (input_l * (1.0 - mix)) + (delayed_l * mix);
            }

            // Channel Right
            if let Some(sample_r) = samples.next() {
                let input_r = *sample_r;
                let delayed_r = linear_interpolate(&self.delay_buffer_r, read_pos);

                if let Some(buf_val) = self.delay_buffer_r.get_mut(self.write_pos) {
                    *buf_val = input_r + (delayed_r * feedback);
                }
                *sample_r = (input_r * (1.0 - mix)) + (delayed_r * mix);
            }

            // 5. Safe Write Position Increment
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

impl Vst3Plugin for TapeDelay {
    const VST3_CLASS_ID: [u8; 16] = *b"TapeDelayPlug123";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Delay, Vst3SubCategory::Modulation, Vst3SubCategory::Fx];
}

#[inline]
fn linear_interpolate(buffer: &[f32], read_pos: f32) -> f32 {
    let index_a = read_pos as usize;
    let index_b = (index_a + 1) % buffer.len();
    let fraction = read_pos - index_a as f32;
    buffer[index_a] * (1.0 - fraction) + buffer[index_b] * fraction
}

nih_export_vst3!(TapeDelay);
