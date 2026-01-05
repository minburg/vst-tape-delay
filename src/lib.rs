use nih_plug::prelude::*;
use nih_plug_vizia::widgets::ParamEvent;
use nih_plug_vizia::ViziaState;
use nih_plug_vizia::{assets, create_vizia_editor, ViziaTheming};
use std::ops::Deref;
// Import Vizia core items (Lens, Model, Context, etc.)
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::param_base::ParamWidgetBase;
use std::borrow::Borrow;
use std::sync::Arc;

pub struct TapeDelay {
    params: Arc<TapeParams>,
    editor_state: Arc<ViziaState>,

    // DSP State
    delay_buffer_l: Vec<f32>,
    delay_buffer_r: Vec<f32>,
    write_pos: usize,
    sample_rate: f32,
    current_delay_samples: f32,
}

// --- FIX 1: Add 'Lens' here so Vizia can see into the struct ---
#[derive(Params, Lens)]
struct TapeParams {
    #[id = "time"]
    pub delay_time_ms: FloatParam,

    #[id = "feedback"]
    pub feedback: FloatParam,

    #[id = "mix"]
    pub mix: FloatParam,
}

// --- FIX 2: Create a Data Model wrapper ---
// This acts as the bridge between your plugin data and the GUI.
#[derive(Lens)]
struct Data {
    params: Arc<TapeParams>,
}

// We implement Model so we can "build" this data into the GUI context.
impl Model for Data {}

impl Default for TapeParams {
    fn default() -> Self {
        Self {
            delay_time_ms: FloatParam::new(
                "Delay Time",
                350.0,
                FloatRange::Linear {
                    min: 10.0,
                    max: 1000.0,
                },
            )
            .with_unit(" ms")
            .with_smoother(SmoothingStyle::Linear(10.0)),
            feedback: FloatParam::new(
                "Feedback",
                0.4,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(1))
            .with_smoother(SmoothingStyle::Linear(10.0)),
            mix: FloatParam::new("Dry/Wet", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(1))
                .with_smoother(SmoothingStyle::Linear(10.0)),
        }
    }
}

impl Default for TapeDelay {
    fn default() -> Self {
        Self {
            params: Arc::new(TapeParams::default()),
            editor_state: ViziaState::new(|| (700, 350)),
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

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[],
        aux_output_ports: &[],
        names: PortNames::const_default(),
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
        buffer_config: &BufferConfig,
        _ctx: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        let max_samples = (self.sample_rate * 2.0) as usize;
        self.delay_buffer_l = vec![0.0; max_samples];
        self.delay_buffer_r = vec![0.0; max_samples];
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let target_delay_samples = self.params.delay_time_ms.value() / 1000.0 * self.sample_rate;
        let smooth_coeff = 0.0005;
        let smooth_anti_coeff = 1.0 - smooth_coeff;
        let buffer_len = self.delay_buffer_l.len();

        for (_, block_frame) in buffer.iter_samples().enumerate() {
            self.current_delay_samples = (smooth_anti_coeff * self.current_delay_samples)
                + (smooth_coeff * target_delay_samples);
            let feedback = self.params.feedback.smoothed.next();
            let mix = self.params.mix.smoothed.next();

            let mut read_pos = self.write_pos as f32 - self.current_delay_samples;
            while read_pos < 0.0 {
                read_pos += buffer_len as f32;
            }

            let mut channels = block_frame.into_iter();
            let sample_l = channels.next().unwrap();
            let sample_r = channels.next().unwrap();
            let input_l = *sample_l;
            let input_r = *sample_r;

            let delayed_l = linear_interpolate(&self.delay_buffer_l, read_pos);
            self.delay_buffer_l[self.write_pos] = input_l + (delayed_l * feedback);
            *sample_l = (input_l * (1.0 - mix)) + (delayed_l * mix);

            let delayed_r = linear_interpolate(&self.delay_buffer_r, read_pos);
            self.delay_buffer_r[self.write_pos] = input_r + (delayed_r * feedback);
            *sample_r = (input_r * (1.0 - mix)) + (delayed_r * mix);

            self.write_pos = (self.write_pos + 1) % buffer_len;
        }
        ProcessStatus::Normal
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        create_vizia_editor(
            self.editor_state.clone(),
            ViziaTheming::Custom,
            move |cx, _| {
                assets::register_noto_sans_light(cx);
                assets::register_noto_sans_bold(cx);

                // --- FIX 3: Initialize the Data Model ---
                // This makes 'Data' available to all widgets below
                Data {
                    params: params.clone(),
                }
                .build(cx);

                build_gui(cx)
            },
        )
    }
}

impl Vst3Plugin for TapeDelay {
    const VST3_CLASS_ID: [u8; 16] = *b"TapeDelayPlug123";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Delay];
}

#[inline]
fn linear_interpolate(buffer: &[f32], read_pos: f32) -> f32 {
    let index_a = read_pos as usize;
    let index_b = (index_a + 1) % buffer.len();
    let fraction = read_pos - index_a as f32;
    buffer[index_a] * (1.0 - fraction) + buffer[index_b] * fraction
}

fn build_gui(cx: &mut Context) {
    let bg_color = Color::rgb(30, 30, 30);
    let accent_red = Color::rgb(168, 27, 27);

    VStack::new(cx, |cx| {
        // --- Header ---
        VStack::new(cx, |cx| {
            Label::new(cx, "TAPE DELAY")
                .font_size(40.0)
                .font_weight(FontWeightKeyword::Bold)
                .color(accent_red);
        })
        .height(Pixels(80.0))
        .child_space(Stretch(1.0));

        // --- Knobs ---
        HStack::new(cx, |cx| {
            create_analog_knob(
                cx,
                "TIME",
                0.5,
                ParamWidgetBase::normalized_value_lens(
                    Data::params.map(|p| &p.delay_time_ms),
                ),
                Data::params.map(|p| &p.delay_time_ms),
            );

            create_analog_knob(
                cx,
                "FEEDBACK",
                0.4,
                ParamWidgetBase::normalized_value_lens(
                    Data::params.map(|p| &p.feedback),
                ),
                Data::params.map(|p| &p.feedback),
            );

            create_analog_knob(
                cx,
                "DRY / WET",
                0.5,
                ParamWidgetBase::normalized_value_lens(
                    Data::params.map(|p| &p.mix),
                ),
                Data::params.map(|p| &p.mix),
            );
        })
        .height(Stretch(1.0))
        .col_between(Pixels(40.0))
        .child_space(Stretch(1.0));
    })
    .width(Pixels(700.0))
    .height(Pixels(350.0))
    .background_color(bg_color);
}

fn create_analog_knob<L, PL, P>(
    cx: &mut Context,
    label: &str,
    default: f32,
    value_lens: L,
    param_lens: PL,
)
where
    L: Lens<Target = f32>,
    PL: Lens<Target = &P> + Send + Sync + 'static,
    P: Param + Sized,
{
    VStack::new(cx, |cx| {
        Knob::new(cx, default, value_lens, false)
            .width(Pixels(80.0))
            .height(Pixels(80.0))
            .on_changing(move |cx, val| {
                let param: &P = param_lens.get(cx);
                cx.emit(ParamEvent::SetParameterNormalized(param, val));
            });

        Label::new(cx, label)
            .color(Color::rgb(180, 180, 180));
    })
        .child_space(Stretch(1.0));
}






nih_export_vst3!(TapeDelay);
