use std::sync::Arc;

use nih_plug::prelude::Editor;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::*;
use nih_plug_vizia::{create_vizia_editor, ViziaState, ViziaTheming};

use crate::ChorusParams;

use self::param_knob::ParamKnob;

mod param_knob;

const STYLE: &str = r#"
.param_knob {
    width: 100px;
    height: 100px;
}

label {
    child-space: 1s;
    font-size: 18;
    color: #9DEEDA;
}

.header-label {
    color: #EAEEED;
}

knob {
    width: 50px;
    height: 50px;
}

knob .track {
    background-color: #54deb2;
}

.param-label {
    color: #EAEEED;
}

.tick {
    background-color: #54deb2;
}

.main-gui {
    background-color: #1E1D1D;
}

"#;

#[derive(Lens)]
struct Data {
    chorus_data: Arc<ChorusParams>
}

impl Model for Data {}

pub(crate) fn default_state() -> Arc<ViziaState> {
    ViziaState::new(|| (350, 350))
}

pub(crate) fn create(
    chorus_data: Arc<ChorusParams>,
    editor_state: Arc<ViziaState>,
) -> Option<Box<dyn Editor>> {
    create_vizia_editor(editor_state,
                        ViziaTheming::Custom, move |cx, _| {

            // cx.add_theme(STYLE);

            Data {
                chorus_data: chorus_data.clone(),
            }.build(cx);

            ResizeHandle::new(cx);
            VStack::new(cx, |cx| {
                Label::new(cx, "MAEROR'S CHORUS")
                    .font_size(24.0)
                    .height(Pixels(75.0))
                    .child_top(Stretch(1.0))
                    .child_bottom(Stretch(1.0))
                    .class("header-label");
                VStack::new(cx, |cx| {
                    HStack::new(cx, |cx| {
                        ParamKnob::new(cx, Data::chorus_data, |params| &params.time, false);
                        ParamKnob::new(cx, Data::chorus_data, |params| &params.feedback, false);
                        ParamKnob::new(cx, Data::chorus_data, |params| &params.mix, false);
                    }).col_between(Pixels(15.0));

                }).col_between(Pixels(30.0));


            }).row_between(Pixels(0.0))
                .child_left(Stretch(1.0))
                .child_right(Stretch(1.0))
                .class("main-gui");
        })
}
