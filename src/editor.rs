use std::sync::Arc;

use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{create_vizia_editor, ViziaState, ViziaTheming};

use crate::TapeParams;

use self::param_knob::ParamKnob;

mod param_knob;

pub const COMFORTAA_LIGHT_TTF: &[u8] = include_bytes!("./resource/Comfortaa-Light.ttf");
pub const COMFORTAA: &str = "Comfortaa";

#[derive(Lens)]
struct Data {
    tape_data: Arc<TapeParams>
}

impl Model for Data {}

pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::new(|| (1200, 800))
}

pub(crate) fn create(
    tape_data: Arc<TapeParams>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state,
                        ViziaTheming::Custom, move |cx, _| {

            cx.add_font_mem(&COMFORTAA_LIGHT_TTF);
            cx.set_default_font(&[COMFORTAA]);
            cx.add_stylesheet(include_style!("/src/resource/style.css"))
                .expect("Failed to add stylesheet");

            Data {
                tape_data: tape_data.clone(),
            }.build(cx);

            VStack::new(cx, |cx| {
                HStack::new(cx, |cx| {
                    Label::new(cx, "CONVOLUTION'S TAPE DELAY")
                        .class("header-title");
                }).child_space(Stretch(1.0))
                    .class("title-section");

                // Wrap knobs in a container that handles the centering
                HStack::new(cx, |cx| {
                    ParamKnob::new(cx, Data::tape_data, |params| &params.delay_time_ms, false)
                        .width(Stretch(1.0));
                    ParamKnob::new(cx, Data::tape_data, |params| &params.feedback, false)
                        .width(Stretch(1.0));
                    ParamKnob::new(cx, Data::tape_data, |params| &params.mix, false)
                        .width(Stretch(1.0));
                }).class("knob-section");
                    // .child_space(Stretch(1.0)); // This pushes all knobs to the center together

            }).class("main-gui");

            ResizeHandle::new(cx);
        })
}
