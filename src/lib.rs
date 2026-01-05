use nih_plug::prelude::*;
use nih_plug_vizia::ViziaState;
use std::sync::Arc;

mod editor;

pub struct ChorusPlugin {
    params: Arc<ChorusParams>,
}

#[derive(Params)]
struct ChorusParams {
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,

    #[id = "time"]
    pub time: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for ChorusPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(ChorusParams::default()),
        }
    }
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),

            time: FloatParam::new("Time", 15.0, FloatRange::Linear { min: 0.1, max: 50.0 })
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

impl Plugin for ChorusPlugin {
    const NAME: &'static str = "Maeror's Chorus";
    const VENDOR: &'static str = "Hubert Łabuda";
    const URL: &'static str = "https://www.linkedin.com/in/hubert-%C5%82abuda/";
    const EMAIL: &'static str = "none";
    const VERSION: &'static str = "none";

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
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        true
    }

    fn reset(&mut self) {
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.params.editor_state.clone(),
        )
    }
}

impl ClapPlugin for ChorusPlugin {
    const CLAP_ID: &'static str = "{{ cookiecutter.clap_id }}";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("{{ cookiecutter.description }}");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo];
}

impl Vst3Plugin for ChorusPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"maeror____Chorus";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Delay, Vst3SubCategory::Modulation, Vst3SubCategory::Fx];
}

nih_export_vst3!(ChorusPlugin);
