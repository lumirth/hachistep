use crate::{
    commands::{Intent, Invoke},
    instruments, lcd,
    player::PlayerView,
    settings, theme, Desktop,
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        description_list::DescriptionList,
        input::{Input, InputEvent, InputState, NumberInput},
        radio::RadioGroup,
        slider::{Slider, SliderEvent, SliderState},
        ActiveTheme, Disableable, Sizable,
    },
    prelude::*,
    *,
};
use hs_runtime::{Command, Player, Presentation};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Motion,
    Diagnostics,
    Link,
    Tcp,
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
        let peer_key = player.presentation().peer.map(|id| (id, kind));
        let existing = cx.global::<Desktop>().tools.get(&key).copied().or_else(|| {
            (kind == Kind::Link)
                .then(|| peer_key.and_then(|key| cx.global::<Desktop>().tools.get(&key).copied()))
                .flatten()
        });
        if existing.is_some_and(|handle| {
            handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        }) {
            return;
        }
        let name = match kind {
            Kind::Motion => "Walking",
            Kind::Diagnostics => "Diagnostics",
            Kind::Link => "Local Link",
            Kind::Tcp => "TCP Connection",
        };
        let filename = player
            .presentation()
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_else(|| "hachistep".into());
        let (width, height) = match kind {
            Kind::Motion => (380., 300.),
            Kind::Diagnostics => (480., 460.),
            Kind::Link => (420., link_height(&player, cx)),
            Kind::Tcp => (420., 320.),
        };
        let mut options = crate::options(&format!("{name} · {filename}"), width, height);
        options.window_min_size = Some(size(
            px(340.),
            px(if kind == Kind::Link { 140. } else { 260. }),
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
            .map(|(bounds, _)| *bounds)
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

fn link_height(player: &Player, cx: &App) -> f32 {
    if player.presentation().peer.is_some() {
        return 280.;
    }
    let count = cx
        .global::<Desktop>()
        .windows
        .iter()
        .filter_map(|w| w.view.upgrade())
        .filter(|view| {
            let view = view.read(cx);
            view.player().id() != player.id() && view.presentation().ready
        })
        .count();
    (90. + count as f32 * 70.).clamp(140., 370.)
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
    automatic_size: Option<Size<Pixels>>,
    menu_state: String,
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
                view.menu_state.clear();
                crate::player_menu(&view.owner, &mut view.menu_state, cx);
            }
        });
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let automatic = cx
            .global::<Desktop>()
            .tool_bounds
            .get(&key)
            .is_none_or(|(_, automatic)| *automatic);
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
                            view.layout(window, cx);
                            if window.is_window_active() {
                                crate::player_menu(&view.owner, &mut view.menu_state, cx);
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
            automatic_size: automatic.then_some(window.bounds().size),
            menu_state: String::new(),
            _subscriptions: vec![
                cadence_changed,
                slider_changed,
                address_changed,
                release,
                activation,
            ],
        }
    }
    fn layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = window.bounds();
        if self.automatic_size.is_some_and(|size| size != bounds.size) {
            self.automatic_size = None;
        }
        if self.kind == Kind::Link && self.automatic_size.is_some() {
            let size = size(bounds.size.width, px(link_height(&self.player, cx)));
            if size != bounds.size {
                window.resize(size);
                self.automatic_size = Some(size);
            }
        }
        cx.global_mut::<Desktop>().tool_bounds.insert(
            (self.player.id(), self.kind),
            (window.bounds(), self.automatic_size.is_some()),
        );
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
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .child("Acceleration")
                    .child(
                        div().flex().gap_3().children(
                            [
                                ("X", cx.theme().chart_1),
                                ("Y", cx.theme().chart_2),
                                ("Z", cx.theme().chart_3),
                            ]
                            .map(|(axis, color)| div().text_color(color).child(axis)),
                        ),
                    ),
            )
            .child(div().h_24().child(instruments::motion(&frame, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child("Cadence")
                    .child(
                        div().w_32().child(
                            NumberInput::new(&self.cadence)
                                .small()
                                .suffix(div().text_xs().child("/min")),
                        ),
                    ),
            )
            .child(div().px_2().child(Slider::new(&self.slider)))
            .child(
                div().flex().justify_end().child(
                    Button::new("walking")
                        .label(if frame.walking {
                            "Stop Walking"
                        } else {
                            "Start Walking"
                        })
                        .on_click(cx.listener(|view, _, _, cx| view.walking(cx))),
                ),
            )
            .into_any_element()
    }
    fn diagnostics(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        let firmware = frame.firmware.clone();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                DescriptionList::horizontal()
                    .columns(1)
                    .bordered(false)
                    .small()
                    .item("Application", env!("CARGO_PKG_VERSION"), 1)
                    .item(
                        "Core revision",
                        hs_runtime::CORE_REVISION
                            .chars()
                            .take(12)
                            .collect::<String>(),
                        1,
                    )
                    .item(
                        "EEPROM",
                        frame
                            .eeprom
                            .as_deref()
                            .map(settings::filename)
                            .unwrap_or_default(),
                        1,
                    )
                    .item(
                        "Firmware",
                        Button::new("copy-firmware")
                            .ghost()
                            .small()
                            .label(format!("{}…", &firmware[..firmware.len().min(12)]))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(firmware.clone()))
                            })
                            .into_any_element(),
                        1,
                    )
                    .item(
                        "Execution",
                        if frame.paused {
                            "Paused".into()
                        } else {
                            format!("{:.1}%", frame.speed * 100.)
                        },
                        1,
                    )
                    .item("Guest time", elapsed(frame.time.raw()), 1)
                    .item(
                        "LCD",
                        if frame.display_enabled {
                            "Enabled"
                        } else {
                            "Off"
                        },
                        1,
                    )
                    .item("Infrared", frame.connection_name(), 1)
                    .item(
                        "Audio buffer",
                        format!(
                            "{:.1} ms",
                            frame.audio_buffered as f64 * 1000. / f64::from(frame.audio_rate)
                        ),
                        1,
                    )
                    .item("Audio underruns", frame.audio_underruns.to_string(), 1),
            )
            .when_some(frame.audio_error, |root, error| {
                root.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("retry-audio")
                            .label("Reconnect Audio")
                            .on_click(|_, _, cx| cx.global::<Desktop>().runtime.retry_audio()),
                    )
                    .child(Button::new("export").label("Export…").on_click(
                        cx.listener(|view, _, _, cx| view.invoke(Intent::DiagnosticsExport, cx)),
                    )),
            )
            .into_any_element()
    }
    fn peer(&self, id: hs_runtime::Id, cx: &App) -> Option<(Player, String)> {
        cx.global::<Desktop>()
            .windows
            .iter()
            .filter_map(|w| w.view.upgrade())
            .find_map(|view| {
                let view = view.read(cx);
                (view.player().id() == id).then(|| (view.player().clone(), view.filename()))
            })
    }
    fn thumbnail(&self, player: &Player, name: String, cx: &App) -> AnyElement {
        let frame = player.presentation();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(div().w(rems(9.)).h_24().child(lcd::display(
                frame.pixels,
                frame.contrast,
                false,
                cx.global::<Desktop>().preferences.palette,
            )))
            .child(div().text_sm().child(name))
            .into_any_element()
    }
    fn activity(&self, frame: &Presentation, cx: &App) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
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
    fn link(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        if let Some(peer) = frame.peer.and_then(|id| self.peer(id, cx)) {
            return div()
                .flex()
                .flex_col()
                .gap_5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            self.thumbnail(
                                &self.player,
                                frame
                                    .eeprom
                                    .as_deref()
                                    .map(settings::filename)
                                    .unwrap_or_default(),
                                cx,
                            ),
                        )
                        .child(div().text_color(cx.theme().primary).child("↔"))
                        .child(self.thumbnail(&peer.0, peer.1, cx)),
                )
                .child(self.activity(&frame, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_sm().child("Linked"))
                        .child(Button::new("disconnect").label("Disconnect").on_click(
                            cx.listener(|view, _, _, _| view.player.send(Command::Disconnect)),
                        )),
                )
                .into_any_element();
        }
        let peers: Vec<_> = cx
            .global::<Desktop>()
            .windows
            .iter()
            .filter_map(|w| w.view.upgrade())
            .filter_map(|view| {
                let view = view.read(cx);
                let other = view.presentation();
                (view.player().id() != self.player.id() && other.ready && !other.closing)
                    .then(|| (view.player().clone(), view.filename()))
            })
            .collect();
        let available = frame.connection == hs_runtime::Connection::Disconnected;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(!available, |root| {
                root.child(
                    div()
                        .text_sm()
                        .child("Disconnect TCP to link a local player."),
                )
            })
            .when(peers.is_empty(), |root| {
                root.child(
                    div()
                        .py_6()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No other players open"),
                )
            })
            .children(peers.into_iter().map(|(player, name)| {
                let other = player.presentation();
                let busy = other.peer.is_some()
                    || other.connection != hs_runtime::Connection::Disconnected;
                let id = player.id();
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .py_2()
                    .child(div().w_24().h_16().child(lcd::display(
                        other.pixels,
                        other.contrast,
                        false,
                        cx.global::<Desktop>().preferences.palette,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(name)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if busy {
                                        "Connected"
                                    } else if other.paused {
                                        "Paused"
                                    } else {
                                        "Available"
                                    }),
                            ),
                    )
                    .child(
                        Button::new(("connect", id))
                            .small()
                            .label("Connect")
                            .disabled(busy || !available || other.paused || frame.paused)
                            .on_click(cx.listener(move |view, _, _, _| {
                                view.player.send(Command::Link(id))
                            })),
                    )
            }))
            .into_any_element()
    }
    fn tcp(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        let busy = frame.connection.is_active();
        let local = frame.peer.is_some();
        div()
            .flex()
            .flex_col()
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
                div()
                    .flex()
                    .flex_col()
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
            .child(self.activity(&frame, cx))
            .child(
                div()
                    .flex()
                    .items_center()
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
            Kind::Diagnostics => self.diagnostics(cx),
            Kind::Link => self.link(cx),
            Kind::Tcp => self.tcp(cx),
        };
        let name = match self.kind {
            Kind::Motion => "Walking",
            Kind::Diagnostics => "Diagnostics",
            Kind::Link => "Local Link",
            Kind::Tcp => "TCP Connection",
        };
        let frame = self.player.presentation();
        let filename = frame
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_default();
        div()
            .id("tool")
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, action: &Invoke, window, cx| {
                if action.intent == Intent::Close {
                    window.remove_window();
                } else {
                    view.invoke(action.intent.clone(), cx);
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
                        div()
                            .flex()
                            .flex_col()
                            .p_5()
                            .gap_4()
                            .child(content)
                            .when_some(
                                self.player.presentation().connection_error,
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
