use crate::nih_error;
use crate::nih_log;
use crate::AtomicF32;
use nih_plug::prelude::{util, Editor};
use nih_plug_vizia::assets::register_noto_sans_light;
use nih_plug_vizia::widgets::ParamButton;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::editor::my_peak_meter::MyPeakMeter;
use crate::TapeParams;
use image::load_from_memory;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::{create_vizia_editor, ViziaState, ViziaTheming};

use self::param_knob::ParamKnob;

mod my_peak_meter;
mod param_knob;

pub const ORBITRON_TTF: &[u8] = include_bytes!("./resource/Orbitron-Regular.ttf");
pub const COMFORTAA_LIGHT_TTF: &[u8] = include_bytes!("./resource/Comfortaa-Light.ttf");
pub const COMFORTAA: &str = "Comfortaa";

const BG_IMAGE_BYTES: &[u8] = include_bytes!("./resource/ghost.png");

#[derive(Lens)]
struct Data {
    tape_data: Arc<TapeParams>,
    peak_meter_l: Arc<AtomicF32>,
    peak_meter_r: Arc<AtomicF32>,
}

impl Model for Data {}

pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::new(|| (1200, 800))
}

pub(crate) fn create(
    tape_data: Arc<TapeParams>,
    peak_meter_l: Arc<AtomicF32>,
    peak_meter_r: Arc<AtomicF32>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state, ViziaTheming::Custom, move |cx, _| {
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
            peak_meter_l: peak_meter_l.clone(),
            peak_meter_r: peak_meter_r.clone(),
        }
        .build(cx);

        VStack::new(cx, |cx| {
            VStack::new(cx, |cx| {
                Label::new(cx, "CONVOLUTION'S TAPE DELAY").class("header-title");
                Label::new(cx, "v0.1.0").class("header-version-title");
            })
            .row_between(Pixels(10.0))
            .child_space(Stretch(1.0))
            .class("title-section");

            HStack::new(cx, |cx| {
                VStack::new(cx, |cx| {
                    MyPeakMeter::new(
                        cx,
                        Data::peak_meter_l.map(|peak_meter_l| {
                            util::gain_to_db(peak_meter_l.load(Ordering::Relaxed))
                        }),
                        Some(Duration::from_millis(30)),
                    )
                    .class("vu-meter-no-text")
                    .width(Stretch(1.0))
                    .height(Pixels(45.0));
                    MyPeakMeter::new(
                        cx,
                        Data::peak_meter_r.map(|peak_meter_r| {
                            util::gain_to_db(peak_meter_r.load(Ordering::Relaxed))
                        }),
                        Some(Duration::from_millis(30)),
                    )
                    .class("vu-meter")
                    .width(Stretch(1.0))
                    .height(Pixels(45.0));
                })
                .height(Stretch(1.0))
                .width(Stretch(1.5));

                // Element::new(cx).width(Stretch(0.1)).height(Stretch(1.0));

                ParamButton::new(cx, Data::tape_data, |params| &params.broken_tape)
                    .width(Stretch(0.45))
                    .height(Stretch(0.6))
                    .child_left(Stretch(1.0))
                    .child_right(Stretch(1.0))
                    .class("broken-button");

                ParamButton::new(cx, Data::tape_data, |params| &params.time_sync)
                    .width(Stretch(0.6))
                    .height(Stretch(0.6))
                    .child_left(Stretch(1.0))
                    .child_right(Stretch(1.0))
                    .class("sync-button");

                ParamButton::new(cx, Data::tape_data, |params| &params.distortion_mode)
                    .width(Stretch(0.6))
                    .height(Stretch(0.6))
                    .child_left(Stretch(1.0))
                    .child_right(Stretch(1.0))
                    .class("distortion-button");
            })
            .width(Stretch(0.8))
            .height(Stretch(0.2))
            .class("meter-section");

            // Wrap knobs in a container that handles the centering
            HStack::new(cx, |cx| {
                VStack::new(cx, |cx| {
                    ParamKnob::new(cx, Data::tape_data, |params| &params.gain, false)
                        .width(Stretch(1.0));
                })
                .class("portion");
                VStack::new(cx, |cx| {
                    ParamKnob::new(cx, Data::tape_data, |params| &params.delay_time_ms, false)
                        .width(Stretch(1.0))
                        .disabled(Data::tape_data.map(|params| params.distortion_mode.value()));
                })
                .class("portion");
                VStack::new(cx, |cx| {
                    ParamKnob::new(cx, Data::tape_data, |params| &params.feedback, false)
                        .width(Stretch(1.0))
                        .disabled(Data::tape_data.map(|params| params.distortion_mode.value()));
                })
                .class("portion");
                VStack::new(cx, |cx| {
                    ParamKnob::new(cx, Data::tape_data, |params| &params.mix, false)
                        .width(Stretch(1.0))
                        .disabled(Data::tape_data.map(|params| params.distortion_mode.value()));
                })
                .class("portion");
            })
            .width(Stretch(0.8))
            .class("knob-section");
        })
        .width(Stretch(1.0))
        .height(Stretch(1.0))
        // .child_space(Pixels(5.0))
        .class("main-gui");
    })
}
