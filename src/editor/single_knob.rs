use nih_plug::prelude::Param;
use nih_plug_vizia::vizia::prelude::*;
use nih_plug_vizia::widgets::param_base::ParamWidgetBase;

#[derive(Debug)]
pub enum SingleKnobEvent {
    BeginSetParam,
    SetParam(f32),
    EndSetParam,
}

#[derive(Lens)]
pub struct SingleKnob {
    param_base: ParamWidgetBase,
}

impl SingleKnob {
    pub fn new<L, Params, P, FMap>(
        cx: &mut Context,
        params: L,
        params_to_param: FMap,
        centered: bool,
    ) -> Handle<'_, Self>
    where
        L: Lens<Target = Params> + Clone + Copy,
        Params: 'static,
        P: Param + 'static,
        FMap: Fn(&Params) -> &P + Copy + 'static,
    {
        Self {
            param_base: ParamWidgetBase::new(cx, params.clone(), params_to_param),
        }
        .build(
            cx,
            ParamWidgetBase::build_view(params, params_to_param, move |cx, param_data| {
                ZStack::new(cx, |cx| {
                    VStack::new(cx, |cx| {
                        Label::new(
                            cx,
                            params.map(move |params| params_to_param(params).name().to_owned()),
                        )
                        .class("single-knob-label");
                        Label::new(
                            cx,
                            params.map(move |params| {
                                params_to_param(params)
                                    .normalized_value_to_string(
                                        params_to_param(params)
                                            .modulated_normalized_value()
                                            .to_owned(),
                                        true,
                                    )
                                    .to_owned()
                            }),
                        )
                        .class("single-knob-label");
                    })
                    .row_between(Pixels(6.0))
                    .child_space(Stretch(1.0));

                    Knob::custom(
                        cx,
                        param_data.param().default_normalized_value(),
                        params.map(move |params| {
                            params_to_param(params).unmodulated_normalized_value()
                        }),
                        move |cx, lens| {
                            // A ZStack allows you to layer a "hit area" background and the visual arc
                            ZStack::new(cx, |cx| {
                                // Transparent "Hit Surface" to capture mouse everywhere
                                Element::new(cx)
                                    .width(Pixels(80.0))
                                    .height(Pixels(80.0))
                                    .class("single-knob-hitbox");

                                // Visual Arc
                                ArcTrack::new(
                                    cx,
                                    centered,
                                    Percentage(330.0),
                                    Percentage(13.2),
                                    -150.,
                                    150.,
                                    KnobMode::Continuous,
                                )
                                .value(lens)
                                .class("single-knob-arc");
                            })
                            .child_space(Stretch(1.0))
                            .width(Pixels(99.0))
                            .height(Pixels(99.0))
                        },
                    )
                    .space(Stretch(1.0))
                    .on_mouse_down(move |cx, _button| {
                        cx.emit(SingleKnobEvent::BeginSetParam);
                    })
                    .on_changing(move |cx, val| {
                        cx.emit(SingleKnobEvent::SetParam(val));
                    })
                    .on_mouse_up(move |cx, _button| {
                        cx.emit(SingleKnobEvent::EndSetParam);
                    });
                })
                .child_space(Stretch(1.0));
            }),
        )
    }
}

impl View for SingleKnob {
    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        event.map(|param_change_event, _| match param_change_event {
            SingleKnobEvent::BeginSetParam => {
                self.param_base.begin_set_parameter(cx);
            }
            SingleKnobEvent::SetParam(val) => {
                self.param_base.set_normalized_value(cx, *val);
            }
            SingleKnobEvent::EndSetParam => {
                self.param_base.end_set_parameter(cx);
            }
        });
    }
}
