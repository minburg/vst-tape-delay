use crate::nih_error;
use crate::nih_log;
use nih_plug_vizia::assets::register_noto_sans_light;
use std::sync::Arc;

use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::{create_vizia_editor, ViziaState, ViziaTheming};
use image::load_from_memory;

use crate::TapeParams;

use self::param_knob::ParamKnob;

mod param_knob;

pub const ORBITRON_TTF: &[u8] = include_bytes!("./resource/Orbitron-Regular.ttf");
pub const COMFORTAA_LIGHT_TTF: &[u8] = include_bytes!("./resource/Comfortaa-Light.ttf");
pub const COMFORTAA: &str = "Comfortaa";

const BG_IMAGE_BYTES: &[u8] = include_bytes!("./resource/ghost.png");

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

            register_noto_sans_light(cx);

            cx.add_font_mem(&COMFORTAA_LIGHT_TTF);
            cx.add_font_mem(&ORBITRON_TTF);
            cx.set_default_font(&[COMFORTAA]);

            match load_from_memory(BG_IMAGE_BYTES) {
                Ok(img) => cx.load_image("ghost.png", img, ImageRetentionPolicy::Forever),
                Err(e) => nih_error!("Failed to load image: {}", e),
            }

            if let Err(e) = cx.add_stylesheet(include_style!("/src/resource/style.css")) {
                nih_log!("CSS Error: {:?}", e);
            }

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

                    VStack::new(cx, |cx| {
                        ParamKnob::new(cx, Data::tape_data, |params| &params.gain, false)
                            .width(Stretch(1.0));
                    }).class("portion");
                    VStack::new(cx, |cx| {
                        ParamKnob::new(cx, Data::tape_data, |params| &params.delay_time_ms, false)
                            .width(Stretch(1.0));
                    }).class("portion");
                    VStack::new(cx, |cx| {
                        ParamKnob::new(cx, Data::tape_data, |params| &params.feedback, false)
                            .width(Stretch(1.0));
                    }).class("portion");
                    VStack::new(cx, |cx| {
                        ParamKnob::new(cx, Data::tape_data, |params| &params.mix, false)
                            .width(Stretch(1.0));
                    }).class("portion");
                }).width(Stretch(1.0)).class("knob-section");

            }).width(Stretch(1.0))
                .height(Stretch(1.0))
                // .child_space(Pixels(5.0))
                .class("main-gui");

        })
}
