use crate::{commands::Intent, instruments, lcd, player::PlayerView, settings, theme, Desktop};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        collapsible::Collapsible,
        h_flex,
        input::{Input, InputEvent, InputState, NumberInput},
        radio::RadioGroup,
        slider::{Slider, SliderEvent, SliderState},
        v_flex, ActiveTheme, Disableable, IconName, Sizable,
    },
    prelude::*,
    *,
};
use hs_runtime::{Command, Player, Presentation};
use std::{collections::BTreeSet, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Motion,
    Link,
    Tcp,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Motion => "Walking",
            Self::Link => "Local Link",
            Self::Tcp => "TCP Connection",
        }
    }
}

#[derive(Clone)]
pub(crate) struct TcpOptions {
    address: String,
    listen: bool,
    remote: bool,
}
impl Default for TcpOptions {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:8081".into(),
            listen: false,
            remote: false,
        }
    }
}

pub(crate) fn open(
    player: Player,
    owner: WeakEntity<PlayerView>,
    kind: Kind,
    parent: &mut Window,
    cx: &mut App,
) {
    let parent_bounds = parent.bounds();
    let screen = parent.display(cx).map(|d| d.bounds());
    cx.defer(move |cx| {
        let key = (player.id(), kind);
        let existing = cx.global::<Desktop>().tools.get(&key).copied();
        if existing.is_some_and(|handle| {
            handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        }) {
            return;
        }
        let name = kind.name();
        let filename = player
            .presentation()
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_else(|| "hachistep".into());
        let (width, height) = match kind {
            Kind::Motion => (392., 340.),
            Kind::Link => (440., 200.),
            Kind::Tcp => (420., 300.),
        };
        let mut options = crate::options(&format!("{name} · {filename}"), width, height);
        options.window_min_size = Some(size(
            px(if kind == Kind::Link { 400. } else { 360. }),
            px(match kind {
                Kind::Link => 180.,
                Kind::Tcp => 260.,
                Kind::Motion => 320.,
            }),
        ));
        let mut origin = parent_bounds.origin + point(parent_bounds.size.width + px(12.), px(0.));
        if let Some(screen) = screen {
            if origin.x + px(width) > screen.right() {
                origin.x = (parent_bounds.origin.x - px(width + 12.)).max(screen.origin.x);
            }
            origin.y = origin
                .y
                .min(screen.bottom() - px(height))
                .max(screen.origin.y);
        }
        let bounds = cx
            .global::<Desktop>()
            .tool_bounds
            .get(&key)
            .copied()
            .unwrap_or_else(|| Bounds::new(origin, size(px(width), px(height))));
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
        match gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Tool::new(player, owner, kind, window, cx))
        }) {
            Ok((handle, _)) => {
                cx.global_mut::<Desktop>().tools.insert(key, handle);
            }
            Err(error) => eprintln!("Could not open {name}: {error:#}"),
        }
    });
}

struct Tool {
    player: Player,
    owner: WeakEntity<PlayerView>,
    kind: Kind,
    cadence: Entity<InputState>,
    slider: Entity<SliderState>,
    address: Entity<InputState>,
    tcp: TcpOptions,
    error: Option<String>,
    focus: FocusHandle,
    details: BTreeSet<hs_runtime::Id>,
    activity_open: bool,
    _subscriptions: Vec<Subscription>,
}
impl Tool {
    fn new(
        player: Player,
        owner: WeakEntity<PlayerView>,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = player.presentation().cadence;
        let cadence = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value.to_string())
                .min(30.)
                .max(240.)
                .step(1.)
        });
        let slider = cx.new(|_| {
            SliderState::new()
                .min(30.)
                .max(240.)
                .step(1.)
                .default_value(f32::from(value))
        });
        let cadence_changed = cx.subscribe_in(
            &cadence,
            window,
            |view, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Ok(value @ 30..=240) = view.cadence.read(cx).value().parse::<u16>() {
                        view.player.send(Command::Cadence(value));
                        view.slider.update(cx, |slider, cx| {
                            slider.set_value(f32::from(value), window, cx)
                        });
                        view.error = None;
                        cx.notify();
                    }
                }
            },
        );
        let slider_changed = cx.subscribe_in(
            &slider,
            window,
            |view, _, event: &SliderEvent, window, cx| {
                if let SliderEvent::Change(value) = event {
                    let value = value.start().round() as u16;
                    view.player.send(Command::Cadence(value));
                    view.cadence.update(cx, |input, cx| {
                        input.set_value(value.to_string(), window, cx)
                    });
                    view.error = None;
                    cx.notify();
                }
            },
        );
        let tcp = cx
            .global::<Desktop>()
            .tcp_options
            .get(&player.id())
            .cloned()
            .unwrap_or_default();
        let address = cx.new(|cx| InputState::new(window, cx).default_value(tcp.address.clone()));
        let address_changed = cx.subscribe(&address, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.tcp.address = view.address.read(cx).value().to_string();
                view.remember_tcp(cx);
            }
        });
        let key = (player.id(), kind);
        let release = cx.on_release(move |_, cx| {
            cx.global_mut::<Desktop>().tools.remove(&key);
        });
        let activation = cx.observe_window_activation(window, |view, window, cx| {
            if window.is_window_active() {
                cx.global_mut::<Desktop>().active_player = Some(view.owner.clone());
                crate::player_menu(&view.owner, cx);
            }
        });
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            if cx
                .update(|window, cx| {
                    view.update(cx, |view, cx| {
                        if view.player.presentation().closed {
                            window.remove_window();
                        } else {
                            cx.global_mut::<Desktop>()
                                .tool_bounds
                                .insert((view.player.id(), view.kind), window.bounds());
                            if window.is_window_active() {
                                crate::player_menu(&view.owner, cx);
                            }
                            cx.notify();
                        }
                    })
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        Self {
            player,
            owner,
            kind,
            cadence,
            slider,
            address,
            tcp,
            error: None,
            focus,
            details: BTreeSet::new(),
            activity_open: false,
            _subscriptions: vec![
                cadence_changed,
                slider_changed,
                address_changed,
                release,
                activation,
            ],
        }
    }
    fn remember_tcp(&self, cx: &mut App) {
        cx.global_mut::<Desktop>()
            .tcp_options
            .insert(self.player.id(), self.tcp.clone());
    }
    fn invoke(&self, intent: Intent, cx: &mut App) {
        crate::invoke_player(&self.owner, intent, cx);
    }
    fn walking(&mut self, cx: &mut Context<Self>) {
        let frame = self.player.presentation();
        if frame.walking {
            self.player.send(Command::Walking(false));
        } else if let Ok(value @ 30..=240) = self.cadence.read(cx).value().parse::<u16>() {
            self.player.send(Command::Cadence(value));
            self.player.send(Command::Walking(true));
        } else {
            self.error = Some("Enter a cadence from 30 to 240 steps/min.".into());
        }
        cx.notify();
    }
    fn motion(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        v_flex()
            .gap_5()
            .child(
                h_flex().justify_between().gap_4().child("Cadence").child(
                    h_flex()
                        .gap_2()
                        .child(div().w_32().child(NumberInput::new(&self.cadence)))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("steps/min"),
                        ),
                ),
            )
            .child(Slider::new(&self.slider))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if frame.walking { "Walking" } else { "Stopped" }),
                    )
                    .child(
                        Button::new("walking")
                            .label(if frame.walking {
                                "Stop Walking"
                            } else {
                                "Start Walking"
                            })
                            .on_click(cx.listener(|view, _, _, cx| view.walking(cx))),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .pt_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .justify_between()
                            .text_sm()
                            .child("Acceleration")
                            .child(
                                h_flex().gap_3().children(
                                    [
                                        ("X", cx.theme().chart_1),
                                        ("Y", cx.theme().chart_2),
                                        ("Z", cx.theme().chart_3),
                                    ]
                                    .map(|(axis, color)| div().text_color(color).child(axis)),
                                ),
                            ),
                    )
                    .child(div().h_20().child(instruments::motion(&frame, cx))),
            )
            .into_any_element()
    }
    fn activity(&self, frame: &Presentation, cx: &App) -> AnyElement {
        h_flex()
            .gap_3()
            .child(
                v_flex()
                    .justify_between()
                    .h_12()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("TX")
                    .child("RX"),
            )
            .child(
                div()
                    .flex_1()
                    .h_12()
                    .child(instruments::infrared(frame, cx)),
            )
            .into_any_element()
    }
    fn activity_disclosure(&self, frame: &Presentation, cx: &mut Context<Self>) -> AnyElement {
        Collapsible::new()
            .open(self.activity_open)
            .child(
                Button::new("activity")
                    .self_start()
                    .px_0()
                    .icon(if self.activity_open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .ghost()
                    .small()
                    .label("Activity")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.activity_open = !view.activity_open;
                        cx.notify();
                    })),
            )
            .content(self.activity(frame, cx))
            .into_any_element()
    }
    fn link(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        let peers: Vec<_> = cx
            .global::<Desktop>()
            .windows
            .iter()
            .filter_map(|w| w.view.upgrade())
            .filter_map(|view| {
                let view = view.read(cx);
                let other = view.presentation();
                (view.player().id() != self.player.id()
                    && other.ready
                    && (!other.closing || frame.peer == Some(view.player().id()))
                    && frame.peer.is_none_or(|id| id == view.player().id()))
                .then(|| (view.player().clone(), view.filename()))
            })
            .collect();
        let available = !frame.connection.is_active();
        v_flex()
            .gap_2()
            .when(frame.paused, |root| {
                root.child(div().text_sm().child("Resume emulation to connect."))
            })
            .when(!available, |root| {
                root.child(div().text_sm().child("Disconnect TCP to link locally."))
            })
            .when(peers.is_empty(), |root| {
                root.child(
                    div()
                        .py_6()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No other emulation windows open"),
                )
            })
            .children(peers.into_iter().map(|(player, name)| {
                let other = player.presentation();
                let id = player.id();
                let linked = frame.peer == Some(id);
                let busy = !linked && (other.peer.is_some() || other.connection.is_active());
                let status = if linked {
                    "Linked".into()
                } else if other.closing {
                    "Closing".into()
                } else if other.peer.is_some() {
                    "Linked locally".into()
                } else if busy {
                    other.connection_name()
                } else if other.paused {
                    "Paused".into()
                } else {
                    "Available".into()
                };
                let open = self.details.contains(&id);
                Collapsible::new()
                    .open(open)
                    .child(
                        h_flex()
                            .gap_4()
                            .py_3()
                            .child(
                                div()
                                    .w(lcd::dimensions(1., 1.).width)
                                    .h(lcd::dimensions(1., 1.).height)
                                    .flex_shrink_0()
                                    .child(lcd::display(
                                        other.pixels,
                                        other.contrast,
                                        false,
                                        crate::palette_for(id, cx),
                                    )),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(div().text_ellipsis().child(name))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(status),
                                    )
                                    .child(
                                        Button::new(("details", id))
                                            .self_start()
                                            .px_0()
                                            .icon(if open {
                                                IconName::ChevronDown
                                            } else {
                                                IconName::ChevronRight
                                            })
                                            .ghost()
                                            .xsmall()
                                            .label("Details")
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                if !view.details.remove(&id) {
                                                    view.details.insert(id);
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                Button::new(("connect", id))
                                    .small()
                                    .w_24()
                                    .label(if linked { "Disconnect" } else { "Connect" })
                                    .disabled(
                                        !linked
                                            && (busy || !available || other.paused || frame.paused),
                                    )
                                    .on_click(cx.listener(move |view, _, _, _| {
                                        view.player.send(if linked {
                                            Command::Disconnect
                                        } else {
                                            Command::Link(id)
                                        })
                                    })),
                            ),
                    )
                    .content(
                        v_flex()
                            .gap_2()
                            .pb_3()
                            .text_sm()
                            .child(format!("Emulated time: {}", elapsed(other.time.raw())))
                            .child(
                                div().text_color(cx.theme().muted_foreground).child(
                                    other
                                        .eeprom
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|| "Untitled EEPROM".into()),
                                ),
                            ),
                    )
                    .into_any_element()
            }))
            .when(frame.peer.is_some(), |root| {
                root.child(self.activity_disclosure(&frame, cx))
            })
            .into_any_element()
    }
    fn tcp(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        let busy = frame.connection.is_active();
        let local = frame.peer.is_some();
        v_flex()
            .gap_5()
            .child(
                RadioGroup::horizontal("tcp-role")
                    .disabled(busy || local)
                    .children(["Connect", "Listen"])
                    .selected_index(Some(usize::from(self.tcp.listen)))
                    .on_change(cx.listener(|view, index, _, cx| {
                        view.tcp.listen = *index == 1;
                        view.remember_tcp(cx);
                        cx.notify();
                    })),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(if self.tcp.listen {
                        "Listen address"
                    } else {
                        "Peer address"
                    })
                    .child(Input::new(&self.address).disabled(busy)),
            )
            .when(self.tcp.listen, |root| {
                root.child(
                    Checkbox::new("remote")
                        .label("Allow remote connections")
                        .checked(self.tcp.remote)
                        .disabled(busy)
                        .on_change(cx.listener(|view, value, _, cx| {
                            view.tcp.remote = *value;
                            view.remember_tcp(cx);
                            cx.notify();
                        })),
                )
            })
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(frame.connection_name()),
                    )
                    .child(
                        Button::new("tcp-action")
                            .label(if busy {
                                if self.tcp.listen
                                    && frame.connection == hs_runtime::Connection::Listening
                                {
                                    "Stop Listening"
                                } else {
                                    "Disconnect"
                                }
                            } else if self.tcp.listen {
                                "Listen"
                            } else {
                                "Connect"
                            })
                            .disabled(local || (frame.paused && !busy))
                            .on_click(cx.listener(move |view, _, _, _| {
                                if busy {
                                    view.player.send(Command::Disconnect);
                                } else {
                                    view.player.send(Command::Tcp {
                                        address: view.tcp.address.clone(),
                                        listen: view.tcp.listen,
                                        allow_remote: view.tcp.remote,
                                    });
                                }
                            })),
                    ),
            )
            .child(self.activity_disclosure(&frame, cx))
            .into_any_element()
    }
}

pub(crate) fn elapsed(raw: u128) -> String {
    let seconds = (raw >> 64) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

impl Render for Tool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.kind {
            Kind::Motion => self.motion(cx),
            Kind::Link => self.link(cx),
            Kind::Tcp => self.tcp(cx),
        };
        let name = self.kind.name();
        let frame = self.player.presentation();
        let filename = frame
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_default();
        v_flex()
            .id("tool")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, action: &Intent, window, cx| {
                if crate::system_action(action, window, cx) {
                    return;
                }
                if *action == Intent::Close {
                    window.remove_window();
                } else {
                    view.invoke(action.clone(), cx);
                }
            }))
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(theme::title(format!("{name} · {filename}"), window, cx))
            .child(
                div()
                    .id("tool-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .p_4()
                            .gap_4()
                            .child(content)
                            .when_some(
                                (self.kind != Kind::Motion)
                                    .then_some(frame.connection_error)
                                    .flatten(),
                                |root, error| {
                                    root.child(
                                        div().text_sm().text_color(cx.theme().danger).child(error),
                                    )
                                },
                            )
                            .when_some(self.error.clone(), |root, error| {
                                root.child(
                                    div().text_sm().text_color(cx.theme().danger).child(error),
                                )
                            }),
                    ),
            )
    }
}
