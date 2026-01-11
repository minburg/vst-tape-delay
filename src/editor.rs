use crate::editor::single_knob::SingleKnob;
use crate::nih_error;
use crate::nih_log;
use crate::AtomicF32;
use crate::BoolParam;
use nih_plug::prelude::Param;
use nih_plug::prelude::{util, Editor};
use nih_plug_vizia::assets::register_noto_sans_light;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::ResizeHandle;
use nih_plug_vizia::widgets::{RawParamEvent,ParamEvent};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::editor::my_peak_meter::MyPeakMeter;
use crate::TapeParams;
use nih_plug_vizia::vizia::image::load_from_memory;
use nih_plug_vizia::{create_vizia_editor, ViziaState, ViziaTheming};

use self::param_knob::ParamKnob;

mod my_peak_meter;
mod param_knob;
mod single_knob;

pub const ORBITRON_TTF: &[u8] = include_bytes!("resource/fonts/Orbitron-Regular.ttf");
pub const COMFORTAA_LIGHT_TTF: &[u8] = include_bytes!("resource/fonts/Comfortaa-Light.ttf");
pub const COMFORTAA: &str = "Comfortaa";

const BG_IMAGE_BYTES: &[u8] = include_bytes!("resource/images/background_image.png");
const INSTA_ICON_BYTES: &[u8] = include_bytes!("resource/images/instagram_icon.png");
const SPOTIFY_ICON_BYTES: &[u8] = include_bytes!("resource/images/spotify_icon.png");

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
            Ok(img) => cx.load_image("background_image.png", img, ImageRetentionPolicy::Forever),
            Err(e) => nih_error!("Failed to load image: {}", e),
        }

        match load_from_memory(INSTA_ICON_BYTES) {
            Ok(img) => cx.load_image("insta.png", img, ImageRetentionPolicy::Forever),
            Err(e) => nih_error!("Failed to load image: {}", e),
        }

        match load_from_memory(SPOTIFY_ICON_BYTES) {
            Ok(img) => cx.load_image("spotify.png", img, ImageRetentionPolicy::Forever),
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
                HStack::new(cx, |cx| {
                    Label::new(cx, "Check for Updates")
                        .class("update-link")
                        .on_press(|_| {
                            if let Err(e) = webbrowser::open("https://github.com/minburg/vst-tape-delay/releases") {
                                nih_log!("Failed to open browser: {}", e);
                            }
                        });
                    Label::new(cx, "v0.1.8").class("header-version-title");
                    Element::new(cx)
                        .class("insta-button")
                        .on_press(|_| {
                            let _ = webbrowser::open("https://www.instagram.com/convolution.official/");
                        });
                    Element::new(cx)
                        .class("spotify-button").opacity(0.5)
                        .on_press(|_| {
                            let _ = webbrowser::open("https://open.spotify.com/artist/7k0eMwQbplT3Zyyy0DalRL?si=aalp-7GQQ2O_cZRodAlsNg");
                        });
                })
                    .width(Stretch(1.0))
                    .child_space(Stretch(1.0))
                    .child_top(Stretch(0.01))
                    .child_bottom(Stretch(0.01))
                    .class("link-section");
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
                    .width(Stretch(1.1));

                HStack::new(cx, |cx| {
                    // Broken
                    create_text_button(
                        cx,
                        "Broken",
                        Data::tape_data.map(|p| p.broken_tape.value()),
                        &tape_data,
                        |p| &p.broken_tape,
                        "broken-param-button",
                        "active",
                    )
                        .width(Stretch(0.45))
                        .height(Stretch(0.5))
                        .child_left(Stretch(1.0))
                        .child_right(Stretch(1.0))
                        .child_top(Stretch(0.08))
                        .child_bottom(Stretch(0.08));

                    // Time Sync
                    create_text_button(
                        cx,
                        "Time Sync",
                        Data::tape_data.map(|p| p.time_sync.value()),
                        &tape_data,
                        |p| &p.time_sync,
                        "sync-param-button",
                        "active",
                    )
                        .width(Stretch(0.6))
                        .height(Stretch(0.5))
                        .child_left(Stretch(1.0))
                        .child_right(Stretch(1.0))
                        .child_top(Stretch(0.08))
                        .child_bottom(Stretch(0.08));

                    // Tape Only
                    create_text_button(
                        cx,
                        "Tape Only",
                        Data::tape_data.map(|p| p.distortion_mode.value()),
                        &tape_data,
                        |p| &p.distortion_mode,
                        "distortion-param-button",
                        "active",
                    )
                        .width(Stretch(0.6))
                        .height(Stretch(0.5))
                        .child_left(Stretch(1.0))
                        .child_right(Stretch(1.0))
                        .child_top(Stretch(0.08))
                        .child_bottom(Stretch(0.08));
                })
                    .height(Stretch(1.0))
                    .width(Stretch(1.5))
                    .col_between(Pixels(15.0))
                    .child_top(Stretch(0.08))
                    .child_bottom(Stretch(0.08));
            })
                .width(Stretch(0.8))
                .height(Stretch(0.25))
                .child_top(Stretch(0.01))
                .child_bottom(Stretch(0.01))
                .class("meter-section");

            HStack::new(cx, |cx| {
                HStack::new(cx, |cx| {
                    SingleKnob::new(cx, Data::tape_data, |params| &params.noise, false)
                        .width(Stretch(1.0));
                    SingleKnob::new(cx, Data::tape_data, |params| &params.crackle, false)
                        .width(Stretch(1.0));
                    SingleKnob::new(cx, Data::tape_data, |params| &params.stereo_width, false)
                        .width(Stretch(1.0));
                })
                    .class("finetune-section-inner");
            })
                .width(Stretch(1.0))
                .height(Stretch(0.4))
                .child_top(Stretch(0.08))
                .child_bottom(Stretch(0.08))
                .class("finetune-section");

            HStack::new(cx, |cx| {
                VStack::new(cx, |cx| {
                    ParamKnob::new(cx, Data::tape_data, |params| &params.gain, false)
                        .width(Stretch(1.0));
                })
                    .class("portion");
                VStack::new(cx, |cx| {
                    // Create a Binding to listen to the Distortion Mode boolean
                    Binding::new(cx, Data::tape_data.map(|p| p.distortion_mode.value()), |cx, is_dist_lens| {

                        // Use .get(cx) to read the boolean value
                        if is_dist_lens.get(cx) {
                            // --- MODE ON: SHOW GHOST KNOB ---
                            // This knob is bound to 'ghost_zero', so it sits at 0.0.
                            // We disable it so the user can't turn it.
                            ParamKnob::new(cx, Data::tape_data, |params| &params.ghost_zero, false)
                                .width(Stretch(1.0))
                                .disabled(true) // Grayed out
                                .class("portion"); // Apply same CSS class for consistent layout
                        } else {
                            // --- MODE OFF: SHOW REAL KNOB ---
                            // This is your original knob bound to 'mix'.
                            // It remembers its position (e.g. 30%).
                            ParamKnob::new(cx, Data::tape_data, |params| &params.delay_time_ms, false)
                                .width(Stretch(1.0));
                        }
                    });
                })
                    .class("portion");
                VStack::new(cx, |cx| {
                    // Create a Binding to listen to the Distortion Mode boolean
                    Binding::new(cx, Data::tape_data.map(|p| p.distortion_mode.value()), |cx, is_dist_lens| {

                        // Use .get(cx) to read the boolean value
                        if is_dist_lens.get(cx) {
                            // --- MODE ON: SHOW GHOST KNOB ---
                            // This knob is bound to 'ghost_zero', so it sits at 0.0.
                            // We disable it so the user can't turn it.
                            ParamKnob::new(cx, Data::tape_data, |params| &params.ghost_zero, false)
                                .width(Stretch(1.0))
                                .disabled(true) // Grayed out
                                .class("portion"); // Apply same CSS class for consistent layout
                        } else {
                            // --- MODE OFF: SHOW REAL KNOB ---
                            // This is your original knob bound to 'mix'.
                            // It remembers its position (e.g. 30%).
                            ParamKnob::new(cx, Data::tape_data, |params| &params.feedback, false)
                                .width(Stretch(1.0));
                        }
                    });
                })
                    .class("portion");
                VStack::new(cx, |cx| {
                    // Create a Binding to listen to the Distortion Mode boolean
                    Binding::new(cx, Data::tape_data.map(|p| p.distortion_mode.value()), |cx, is_dist_lens| {

                        // Use .get(cx) to read the boolean value
                        if is_dist_lens.get(cx) {
                            // --- MODE ON: SHOW GHOST KNOB ---
                            // This knob is bound to 'ghost_zero', so it sits at 0.0.
                            // We disable it so the user can't turn it.
                            ParamKnob::new(cx, Data::tape_data, |params| &params.ghost_zero, false)
                                .width(Stretch(1.0))
                                .disabled(true) // Grayed out
                                .class("portion"); // Apply same CSS class for consistent layout
                        } else {
                            // --- MODE OFF: SHOW REAL KNOB ---
                            // This is your original knob bound to 'mix'.
                            // It remembers its position (e.g. 30%).
                            ParamKnob::new(cx, Data::tape_data, |params| &params.mix, false)
                                .width(Stretch(1.0));
                        }
                    });
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

        ResizeHandle::new(cx);
    })
}

pub fn create_text_button<'a, T, L, F>(
    cx: &'a mut Context,
    label_text: &'static str,
    lens: L,
    params: &Arc<T>,
    selector: F,
    class: &str,
    toggle_class: &str,
) -> Handle<'a, DebugWrapper>
where
    T: 'static + Send + Sync,
    L: Lens<Target = bool> + Copy + 'static + Send + Sync,
    F: 'static + Clone + Fn(&T) -> &BoolParam + Send + Sync,
{
    let params_arc = params.clone();
    let selector = selector.clone();

    DebugWrapper::new(cx, label_text, move |cx| {
        Label::new(cx, label_text).hoverable(false);
    })
        .class(class)
        .toggle_class(toggle_class, lens)
        .focusable(true)
        .on_press(move |cx| {
            cx.focus();

            let param = selector(&params_arc);
            let new_value = !param.value();
            let normalized = if new_value { 1.0 } else { 0.0 };

            // --- 1. PREPARE THE DATA ---
            let ptr = param.as_ptr();
            let param_static: &'static BoolParam = unsafe { std::mem::transmute(param) };

            // --- 2. SEND STANDARD EVENTS (FOR HOSTS/VST3) ---
            cx.emit(ParamEvent::BeginSetParameter(param_static));
            cx.emit(ParamEvent::SetParameterNormalized(param_static, normalized));
            cx.emit(ParamEvent::EndSetParameter(param_static));

            // --- 3. SEND RAW EVENTS (FOR STANDALONE/INTERNAL WRAPPERS) ---
            cx.emit(RawParamEvent::BeginSetParameter(ptr));
            cx.emit(RawParamEvent::SetParameterNormalized(ptr, normalized));
            cx.emit(RawParamEvent::EndSetParameter(ptr));

            nih_log!("SUCCESS: [{}] Sent BOTH event types.", label_text);
        })
}

pub struct DebugWrapper {
    name: String,
}

impl DebugWrapper {
    // FIX: Added lifetime 'a to tie the Handle to the Context
    pub fn new<'a, F>(cx: &'a mut Context, name: &str, content: F) -> Handle<'a, Self>
    where
        F: FnOnce(&mut Context),
    {
        Self {
            name: name.to_string(),
        }
            .build(cx, |cx| {
                (content)(cx);
            })
    }
}

impl View for DebugWrapper {
    fn element(&self) -> Option<&'static str> {
        Some("debug-wrapper")
    }

    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|window_event, _| {
            match window_event {
                WindowEvent::MouseEnter => {
                    nih_log!("[{}] Mouse ENTER. Bounds: {:?}", self.name, cx.bounds());
                }
                WindowEvent::MouseLeave => {
                    nih_log!("[{}] Mouse LEAVE", self.name);
                }
                WindowEvent::MouseDown(btn) => {
                    let mouse = cx.mouse();
                    nih_log!(
                        "[{}] Mouse DOWN ({:?}). \n\t-> Mouse Pos: ({}, {})\n\t-> Btn Bounds: ({}, {}, {}, {})",
                        self.name,
                        btn,
                        mouse.cursorx,
                        mouse.cursory,
                        cx.bounds().x, cx.bounds().y, cx.bounds().w, cx.bounds().h
                    );
                }
                WindowEvent::MouseUp(btn) => {
                    nih_log!("[{}] Mouse UP ({:?})", self.name, btn);
                }
                WindowEvent::FocusIn => {
                    nih_log!("[{}] Focus GAINED", self.name);
                }
                WindowEvent::FocusOut => {
                    nih_log!("[{}] Focus LOST", self.name);
                }
                _ => {}
            }
        });
    }
}
