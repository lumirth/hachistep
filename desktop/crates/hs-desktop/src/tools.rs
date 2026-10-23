use crate::{commands::Intent, instruments, lcd, player::PlayerView, settings, theme, Desktop};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        dialog::DialogButtonProps,
        h_flex,
        input::{Input, InputEvent, InputState},
        radio::RadioGroup,
        slider::{Slider, SliderEvent, SliderState},
        switch::Switch,
        tab::{Tab, TabBar},
        v_flex, ActiveTheme, Disableable, Selectable, Sizable, WindowExt,
    },
    prelude::*,
    *,
};
use hs_runtime::{Command, Connection, Player, Presentation};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Motion,
    Infrared,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Motion => "Walking",
            Self::Infrared => "Infrared",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    MelonDs,
    Local,
    Tcp,
}
impl Target {
    fn name(self) -> &'static str {
        match self {
            Self::MelonDs => "melonDS",
            Self::Local => "Local Link",
            Self::Tcp => "TCP",
        }
    }
}
#[derive(Clone)]
pub(crate) struct TcpOptions {
    address: String,
    listen: bool,
    remote: bool,
    target: Target,
}
impl Default for TcpOptions {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:8081".into(),
            listen: false,
            remote: false,
            target: Target::MelonDs,
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
    let screen = parent.display(cx).map(|display| display.bounds());
    cx.defer(move |cx| {
        let key = (player.id(), kind);
        if cx
            .global::<Desktop>()
            .tools
            .get(&key)
            .copied()
            .is_some_and(|handle| {
                handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_ok()
            })
        {
            return;
        }
        let filename = player
            .presentation()
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_else(|| "Untitled EEPROM".into());
        let (width, height) = if kind == Kind::Motion {
            (412., 260.)
        } else {
            (440., 380.)
        };
        let mut options = crate::options(&format!("{} · {filename}", kind.name()), width, height);
        options.window_min_size = Some(size(
            px(390.),
            px(if kind == Kind::Motion { 250. } else { 360. }),
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
            Err(error) => eprintln!("Could not open {}: {error:#}", kind.name()),
        }
    });
}

struct Tool {
    player: Player,
    owner: WeakEntity<PlayerView>,
    kind: Kind,
    target: Target,
    pending_target: Option<Target>,
    cadence: Entity<InputState>,
    slider: Entity<SliderState>,
    address: Entity<InputState>,
    port: Entity<InputState>,
    tcp: TcpOptions,
    error: Option<String>,
    walk_held: Option<bool>,
    focus: FocusHandle,
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
        let frame = player.presentation();
        let target = if frame.peer.is_some() {
            Target::Local
        } else if frame.connection.is_active() && !frame.melon_ds {
            Target::Tcp
        } else {
            cx.global::<Desktop>()
                .tcp_options
                .get(&player.id())
                .map_or(Target::MelonDs, |options| options.target)
        };
        let cadence = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(frame.cadence.to_string())
                .validate(|text, _| text.len() <= 3 && text.bytes().all(|c| c.is_ascii_digit()))
                .min(30.)
                .max(240.)
                .step(1.)
        });
        let slider = cx.new(|_| {
            SliderState::new()
                .min(30.)
                .max(240.)
                .step(1.)
                .default_value(f32::from(frame.cadence))
        });
        let cadence_changed = cx.subscribe_in(
            &cadence,
            window,
            |view, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Ok(value @ 30..=240) = input.read(cx).value().parse::<u16>() {
                        view.invoke(Intent::Cadence(value), cx);
                        if view.slider.read(cx).value().start().round() as u16 != value {
                            view.slider.update(cx, |slider, cx| {
                                slider.set_value(f32::from(value), window, cx)
                            });
                        }
                        cx.notify();
                    }
                } else if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    let pace = input
                        .read(cx)
                        .value()
                        .parse::<u16>()
                        .map(|pace| pace.clamp(30, 240))
                        .unwrap_or(view.player.presentation().cadence);
                    view.invoke(Intent::Cadence(pace), cx);
                    view.slider.update(cx, |slider, cx| {
                        slider.set_value(f32::from(pace), window, cx)
                    });
                    input.update(cx, |input, cx| {
                        input.set_value(pace.to_string(), window, cx)
                    });
                }
            },
        );
        let slider_changed = cx.subscribe_in(
            &slider,
            window,
            |view, _, event: &SliderEvent, window, cx| {
                if let SliderEvent::Change(value) = event {
                    let value = value.start().round() as u16;
                    view.invoke(Intent::Cadence(value), cx);
                    if view.cadence.read(cx).value().as_ref() != value.to_string() {
                        view.cadence.update(cx, |input, cx| {
                            input.set_value(value.to_string(), window, cx)
                        });
                    }
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
        let port_value = cx
            .global::<Desktop>()
            .melon_ports
            .get(&player.id())
            .copied()
            .unwrap_or(3333);
        let port = cx.new(|cx| InputState::new(window, cx).default_value(port_value.to_string()));
        let address_changed = cx.subscribe(&address, |view, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.tcp.address = input.read(cx).value().to_string();
                view.remember_tcp(cx);
                view.error = None;
                cx.notify();
            }
        });
        let port_changed = cx.subscribe(&port, |view, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.error = None;
                if let Ok(port @ 1..=65535) = input.read(cx).value().trim().parse::<u16>() {
                    cx.global_mut::<Desktop>()
                        .melon_ports
                        .insert(view.player.id(), port);
                }
                cx.notify();
            }
        });
        let key = (player.id(), kind);
        let release = cx.on_release(move |view, cx| {
            view.release_walk();
            cx.global_mut::<Desktop>().tools.remove(&key);
        });
        let activation = cx.observe_window_activation(window, |view, window, cx| {
            if window.is_window_active() {
                cx.global_mut::<Desktop>().active_player = Some(view.owner.clone());
                crate::player_menu(&view.owner, cx);
            } else {
                view.release_walk();
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
                        let frame = view.player.presentation();
                        if frame.closed {
                            window.remove_window();
                            return;
                        }
                        cx.global_mut::<Desktop>()
                            .tool_bounds
                            .insert((view.player.id(), view.kind), window.bounds());
                        if !frame.is_connection_active() {
                            if let Some(target) = view.pending_target.take() {
                                view.select_target(target, cx);
                            }
                        }
                        if frame.peer.is_some() {
                            view.target = Target::Local;
                        } else if frame.connection.is_active() {
                            view.target = if frame.melon_ds {
                                Target::MelonDs
                            } else {
                                Target::Tcp
                            };
                        }
                        if view.kind == Kind::Motion
                            && view.cadence.read(cx).value().parse::<u16>().ok()
                                != Some(frame.cadence)
                            && !view.cadence.read(cx).focus_handle(cx).is_focused(window)
                        {
                            view.cadence.update(cx, |input, cx| {
                                input.set_value(frame.cadence.to_string(), window, cx)
                            });
                            view.slider.update(cx, |slider, cx| {
                                slider.set_value(f32::from(frame.cadence), window, cx)
                            });
                        }
                        if window.is_window_active() {
                            crate::player_menu(&view.owner, cx);
                        }
                        cx.notify();
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
            target,
            pending_target: None,
            cadence,
            slider,
            address,
            port,
            tcp,
            error: None,
            walk_held: None,
            focus,
            _subscriptions: vec![
                cadence_changed,
                slider_changed,
                address_changed,
                port_changed,
                release,
                activation,
            ],
        }
    }
    fn release_walk(&mut self) {
        if let Some(walking) = self.walk_held.take() {
            self.player.send(Command::Walking(walking));
        }
    }
    fn invoke(&self, intent: Intent, cx: &mut App) {
        crate::invoke_player(&self.owner, intent, cx);
    }
    fn remember_tcp(&self, cx: &mut App) {
        cx.global_mut::<Desktop>()
            .tcp_options
            .insert(self.player.id(), self.tcp.clone());
    }
    fn motion(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        let hold = cx.global::<Desktop>().preferences.walk_key.clone();
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Switch::new("walking")
                            .checked(frame.walking)
                            .accessibility_label("Walk")
                            .on_change(
                                cx.listener(|view, _, _, cx| {
                                    view.invoke(Intent::ToggleWalking, cx)
                                }),
                            ),
                    )
                    .child(if frame.paused && frame.walking {
                        "Walking (paused)"
                    } else if frame.walking {
                        "Walking"
                    } else {
                        "Not walking"
                    })
                    .child(div().flex_1())
                    .child(
                        div().w_16().child(
                            Input::new(&self.cadence)
                                .small()
                                .text_center()
                                .aria_label("Walking pace in steps per minute"),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("steps/min"),
                    ),
            )
            .child(
                v_flex().child(Slider::new(&self.slider)).child(
                    div()
                        .relative()
                        .mt(-px(4.))
                        .h_8()
                        .children([80, 100, 120, 140, 160].map(|pace| {
                            v_flex()
                                .absolute()
                                .left(relative((pace - 30) as f32 / 210.))
                                .ml(-rems(1.25))
                                .w_10()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .w(px(1.))
                                        .h_1()
                                        .bg(cx.theme().muted_foreground)
                                        .opacity(0.5),
                                )
                                .child(
                                    Button::new(("pace", pace as usize))
                                        .ghost()
                                        .xsmall()
                                        .w_full()
                                        .label(pace.to_string())
                                        .selected(frame.cadence == pace)
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.cadence.update(cx, |input, cx| {
                                                input.set_value(pace.to_string(), window, cx)
                                            });
                                            view.slider.update(cx, |slider, cx| {
                                                slider.set_value(f32::from(pace), window, cx)
                                            });
                                            view.invoke(Intent::Cadence(pace), cx)
                                        })),
                                )
                        })),
                ),
            )
            .child(div().h_8().child(instruments::motion(&frame, cx)))
            .when(!hold.is_empty(), |root| {
                root.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Hold {} to walk.", settings::key_name(&hold))),
                )
            })
            .into_any_element()
    }
    fn setup(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, move |dialog, window, cx| {
            let steps = [
                (
                    "1",
                    "Enable the ARM9 GDB stub.",
                    "Config → Emu settings → Devtools",
                ),
                (
                    "2",
                    "Disable the JIT recompiler.",
                    "Config → Emu settings → CPU emulation",
                ),
                ("3", "Restart melonDS.", ""),
            ]
            .into_iter()
            .map(|(number, action, path)| {
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .size_6()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(cx.theme().muted)
                            .child(number),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(action)
                            .when(!path.is_empty(), |root| {
                                root.child(
                                    div().text_color(cx.theme().muted_foreground).child(path),
                                )
                            }),
                    )
            });
            dialog
                .title("One-time melonDS setup")
                .width(window.rem_size() * 27.)
                .child(v_flex().gap_4().text_sm().children(steps))
                .child(
                    Button::new("setup-done")
                        .small()
                        .self_end()
                        .label("Done")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
        })
    }
    fn toggle(&mut self, cx: &mut Context<Self>) {
        let frame = self.player.presentation();
        if frame.is_connection_active() {
            self.player.send(Command::Disconnect);
            return;
        }
        if frame.paused {
            return;
        }
        self.error = None;
        match self.target {
            Target::MelonDs => match self.port.read(cx).value().trim().parse::<u16>() {
                Ok(port @ 1..=65535) => {
                    let address = format!("127.0.0.1:{port}");
                    let owner = cx
                        .global::<Desktop>()
                        .windows
                        .iter()
                        .filter_map(|window| window.view.upgrade())
                        .find_map(|view| {
                            let view = view.read(cx);
                            let other = view.presentation();
                            (view.player().id() != self.player.id()
                                && other.melon_ds
                                && other.is_connection_active()
                                && other.connection_address.as_ref() == Some(&address))
                            .then(|| view.filename())
                        });
                    if let Some(owner) = owner {
                        self.error = Some(format!(
                            "melonDS is connected to {owner}. Disconnect that walker first."
                        ));
                    } else {
                        self.player.send(Command::MelonDs { port });
                    }
                }
                _ => self.error = Some("Enter an ARM9 debugger port between 1 and 65535.".into()),
            },
            Target::Tcp => self.player.send(Command::Tcp {
                address: self.tcp.address.clone(),
                listen: self.tcp.listen,
                allow_remote: self.tcp.remote,
            }),
            Target::Local => (),
        }
        cx.notify();
    }
    fn select_target(&mut self, target: Target, cx: &mut Context<Self>) {
        self.pending_target = None;
        self.target = target;
        self.tcp.target = target;
        self.remember_tcp(cx);
        self.error = None;
        cx.notify();
    }
    fn change_target(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if target == self.target {
            self.pending_target = None;
            cx.notify();
            return;
        }
        let frame = self.player.presentation();
        if !frame.is_connection_active() {
            self.player.send(Command::Disconnect);
            self.select_target(target, cx);
            return;
        }
        if self.pending_target.is_some() || frame.connection == Connection::Disconnecting {
            self.pending_target = Some(target);
            cx.notify();
            return;
        }
        let action = if frame.peer.is_some() {
            "Unlink and Switch"
        } else if frame.connection == Connection::Listening {
            "Stop Listening and Switch"
        } else if matches!(
            frame.connection,
            Connection::Connecting | Connection::Preparing
        ) {
            "Stop and Switch"
        } else {
            "Disconnect and Switch"
        };
        let owner = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, window, _| {
            let owner = owner.clone();
            dialog
                .title(format!("Switch to {}?", target.name()))
                .width(window.rem_size() * 26.)
                .child("Any transfer in progress will stop.")
                .confirm()
                .button_props(DialogButtonProps::default().ok_text(action))
                .on_ok(move |_, _, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        if view.player.presentation().is_connection_active() {
                            view.pending_target = Some(target);
                        } else {
                            view.select_target(target, cx);
                        }
                        view.player.send(Command::Disconnect);
                        cx.notify();
                    });
                    true
                })
        });
    }
    fn local(&self, frame: &Presentation, cx: &mut Context<Self>) -> AnyElement {
        let peers: Vec<_> = cx
            .global::<Desktop>()
            .windows
            .iter()
            .filter_map(|w| w.view.upgrade())
            .filter_map(|view| {
                let view = view.read(cx);
                let other = view.player().presentation();
                (view.player().id() != self.player.id() && other.ready && !other.closing).then(
                    || {
                        (
                            view.player().id(),
                            view.filename(),
                            other,
                            view.palette(cx),
                            view.raw_display(),
                        )
                    },
                )
            })
            .collect();
        v_flex()
            .gap_2()
            .when(peers.is_empty(), |root| {
                root.child(
                    h_flex()
                        .gap_3()
                        .child(div().flex_1().child("No other walkers are open"))
                        .child(
                            Button::new("new-window")
                                .small()
                                .label("New Window")
                                .on_click(|_, _, cx| crate::new_window(cx, None)),
                        ),
                )
            })
            .children(peers.into_iter().map(|(id, name, other, palette, raw)| {
                let linked = frame.peer == Some(id);
                let blocked = !linked
                    && (frame.is_connection_active()
                        || other.is_connection_active()
                        || frame.paused
                        || other.paused);
                let status = if linked {
                    "Linked"
                } else if other.paused {
                    "Paused"
                } else if other.is_connection_active() {
                    "Connected elsewhere"
                } else if frame.paused {
                    "Resume this walker to link"
                } else {
                    "Available"
                };
                h_flex()
                    .gap_3()
                    .py_2()
                    .child(
                        div()
                            .w(lcd::dimensions(0.75, 1.).width)
                            .h(lcd::dimensions(0.75, 1.).height)
                            .flex_shrink_0()
                            .child(lcd::display(other.pixels, other.contrast, raw, palette)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().whitespace_normal().child(name))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(status),
                            ),
                    )
                    .child(
                        Button::new(("link", id))
                            .small()
                            .w_24()
                            .label(if linked { "Unlink" } else { "Link" })
                            .disabled(blocked)
                            .on_click(cx.listener(move |view, _, _, _| {
                                view.player.send(if linked {
                                    Command::Disconnect
                                } else {
                                    Command::Link(id)
                                })
                            })),
                    )
            }))
            .into_any_element()
    }
    fn infrared(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        let active = frame.is_connection_active();
        let connected = frame.peer.is_some() || frame.connection == Connection::Connected;
        let stopping = frame.connection == Connection::Disconnecting;
        let error = self
            .error
            .clone()
            .or(frame.connection_error.clone())
            .or_else(|| match &frame.connection {
                Connection::Error(error) => Some(error.clone()),
                _ => None,
            });
        let failed = error.is_some();
        let peer = frame.peer.and_then(|id| {
            cx.global::<Desktop>()
                .windows
                .iter()
                .filter_map(|w| w.view.upgrade())
                .find_map(|view| {
                    let view = view.read(cx);
                    (view.player().id() == id).then(|| view.filename())
                })
        });
        let endpoint = match self.target {
            Target::MelonDs => "melonDS".into(),
            Target::Local => peer.clone().unwrap_or_else(|| "Another walker".into()),
            Target::Tcp => frame
                .connection_address
                .clone()
                .filter(|_| active && !frame.melon_ds)
                .unwrap_or_else(|| self.tcp.address.clone()),
        };
        let name = frame
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_else(|| "Untitled EEPROM".into());
        let recent = connected
            && frame.activity.last().is_some_and(|sample| {
                frame.time.raw().saturating_sub(sample.at.raw()) < (1u128 << 64)
            });
        let (headline, guidance) = if failed {
            ("Connection error", "")
        } else if stopping {
            ("Disconnecting…", "")
        } else if frame.connection == Connection::Preparing {
            (
                "Preparing game…",
                "Keep HeartGold or SoulSilver running in melonDS.",
            )
        } else if frame.connection == Connection::Connecting {
            ("Connecting…", "")
        } else if frame.connection == Connection::Listening {
            ("Listening", "Waiting for the other emulator to connect.")
        } else if frame.paused {
            ("Paused", "Resume this walker to connect.")
        } else if recent {
            ("Infrared activity", "")
        } else if connected {
            ("Ready", match self.target {
                Target::MelonDs => "In the game, choose Connect to Pokéwalker. Follow its prompts, then connect from your walker.",
                Target::Local => "Start the connection on both walkers.", Target::Tcp => "Start the connection on both emulators.",
            })
        } else {
            ("Not connected", match self.target { Target::MelonDs => "Start HeartGold or SoulSilver in melonDS. Connect HachiStep before choosing Connect to Pokéwalker in the game.", Target::Local => "Choose another open walker.", Target::Tcp => "" })
        };
        let action = if stopping || connected {
            "Disconnect"
        } else if active {
            if frame.connection == Connection::Listening {
                "Stop Listening"
            } else {
                "Cancel"
            }
        } else if failed {
            "Retry"
        } else if self.target == Target::Tcp && self.tcp.listen {
            "Listen"
        } else {
            "Connect"
        };
        let center = settings::key_name(&cx.global::<Desktop>().preferences.keys[1]);
        let targets = [Target::MelonDs, Target::Local, Target::Tcp];
        let diagram = h_flex()
            .items_start()
            .gap_2()
            .child(
                v_flex()
                    .w_32()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(
                        svg()
                            .path("infrared-walker.svg")
                            .size_10()
                            .text_color(cx.theme().foreground),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_10()
                    .child(instruments::infrared_link(&frame, connected, failed, cx)),
            )
            .child(
                v_flex()
                    .w_32()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .h_10()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(self.target == Target::MelonDs, |root| {
                                root.child(
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            div()
                                                .w_7()
                                                .h_4()
                                                .rounded_sm()
                                                .border_1()
                                                .border_color(cx.theme().foreground),
                                        )
                                        .child(
                                            div()
                                                .w_7()
                                                .h_4()
                                                .rounded_sm()
                                                .border_1()
                                                .border_color(cx.theme().foreground),
                                        ),
                                )
                            })
                            .when(self.target == Target::Local, |root| {
                                root.child(
                                    svg()
                                        .path("infrared-walker.svg")
                                        .size_10()
                                        .text_color(cx.theme().foreground),
                                )
                            })
                            .when(self.target == Target::Tcp, |root| {
                                root.child(
                                    component::Icon::new(gpui_kit::assets::IconName::Network)
                                        .large(),
                                )
                            })
                            .opacity(if active || connected { 1. } else { 0.45 }),
                    ),
            );
        let labels = h_flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .w_32()
                    .flex_shrink_0()
                    .text_center()
                    .whitespace_normal()
                    .child(name),
            )
            .child(div().flex_1().min_w_0())
            .child(
                v_flex().w_32().flex_shrink_0().text_center().child(
                    div()
                        .whitespace_normal()
                        .child(if self.target == Target::Tcp {
                            "TCP".into()
                        } else {
                            endpoint.clone()
                        }),
                ),
            );
        v_flex().size_full().gap_3()
            .child(TabBar::new("infrared-target").segmented().small()
                .selected_index(targets.iter().position(|target| *target == self.target).unwrap_or(0))
                .children(targets.into_iter().map(|target| Tab::new().label(target.name()).flex_1()))
                .on_click(cx.listener(move |view, ix: &usize, window, cx| {
                    view.change_target(targets[*ix], window, cx);
                })))
            .child(div().id("infrared-body").w_full().min_w_0().flex_1().min_h_0().overflow_y_scroll()
                .child(v_flex().w_full().min_w_0().gap_3()
                    .child(v_flex().gap_2().child(diagram).child(labels)
                        .child(div().h_7().child(instruments::infrared_history(&frame, cx)))
                        .child(h_flex().justify_between().text_xs().text_color(cx.theme().muted_foreground)
                            .child(div().text_color(cx.theme().chart_1).child("Sent"))
                            .child("Last 12 seconds")
                            .child(div().text_color(cx.theme().chart_2).child("Received"))))
                    .child(v_flex().w_full().min_w_0().gap_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(if failed { cx.theme().danger } else { cx.theme().foreground }).child(headline))
                        .when_some(self.pending_target, |root, target| root.child(div().text_color(cx.theme().muted_foreground).child(format!("Switching to {}…", target.name()))))
                        .when(self.target == Target::Tcp && active, |root| root.child(div().text_xs().text_color(cx.theme().muted_foreground).child(endpoint.clone())))
                        .when(!guidance.is_empty(), |root| root.child(div().w_full().whitespace_normal().text_color(cx.theme().muted_foreground).child(guidance)))
                        .when_some(error, |root, error| root.child(div().w_full().whitespace_normal().text_color(cx.theme().danger).child(error))))
                    .when(self.target == Target::Local, |root| root.child(self.local(&frame, cx)))
                    .when(self.target == Target::MelonDs && !active, |root| root.child(h_flex().gap_2()
                        .child(div().text_color(cx.theme().muted_foreground).child("ARM9 port"))
                        .child(div().w_16().child(Input::new(&self.port).small().disabled(active)))
                        .child(div().flex_1())
                        .child(Button::new("melonds-setup").ghost().small().label("melonDS setup…")
                            .on_click(cx.listener(|view, _, window, cx| view.setup(window, cx))))))
                    .when(self.target == Target::Tcp, |root| root.child(self.tcp_fields(&frame, cx)))))
            .child(h_flex().gap_3()
                .when(connected, |row| row.child(Button::new("focus-walker").small().label("Show Walker")
                    .tooltip(format!("Press {center} on the home screen, or select Connect from the walker's menu."))
                    .on_click(cx.listener(|view, _, _, cx| crate::focus_player(&view.owner, cx)))))
                .child(div().flex_1())
                .when(self.target != Target::Local, |row| row.child(Button::new("connect").small().w_32().label(action)
                    .when(!active, |button| button.primary())
                    .disabled(stopping || (frame.paused && !active))
                    .on_click(cx.listener(|view, _, _, cx| view.toggle(cx))))))
            .into_any_element()
    }
    fn tcp_fields(&self, frame: &Presentation, cx: &mut Context<Self>) -> AnyElement {
        let active = frame.is_connection_active();
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        RadioGroup::horizontal("tcp-role")
                            .disabled(active)
                            .children(["Connect", "Listen"])
                            .selected_index(Some(usize::from(self.tcp.listen)))
                            .on_change(cx.listener(|view, ix: &usize, _, cx| {
                                view.tcp.listen = *ix == 1;
                                view.remember_tcp(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.address).small().disabled(active)),
                    ),
            )
            .when(self.tcp.listen, |root| {
                root.child(
                    Checkbox::new("allow-remote")
                        .label("Accept connections from other computers")
                        .checked(self.tcp.remote)
                        .disabled(active)
                        .on_change(cx.listener(|view, remote: &bool, window, cx| {
                            view.tcp.remote = *remote;
                            if let Ok(address) = view.tcp.address.parse::<std::net::SocketAddr>() {
                                let address = std::net::SocketAddr::new(
                                    if *remote {
                                        std::net::Ipv4Addr::UNSPECIFIED.into()
                                    } else {
                                        std::net::Ipv4Addr::LOCALHOST.into()
                                    },
                                    address.port(),
                                );
                                view.tcp.address = address.to_string();
                                view.address.update(cx, |field, cx| {
                                    field.set_value(address.to_string(), window, cx)
                                });
                            }
                            view.remember_tcp(cx);
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                    "PocketWalker listens on port 8081 by default. For melonDS-IR, choose Listen.",
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
        let frame = self.player.presentation();
        let filename = frame
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_else(|| "Untitled EEPROM".into());
        let title = format!("{} · {filename}", self.kind.name());
        window.set_window_title(&title);
        let content = if self.kind == Kind::Motion {
            self.motion(cx)
        } else {
            self.infrared(cx)
        };
        v_flex()
            .id("tool")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .key_context("EmulatorCommands")
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if view.kind == Kind::Motion
                    && !event.is_held
                    && !window.has_focused_input(cx)
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && event.keystroke.key == cx.global::<Desktop>().preferences.walk_key
                {
                    if view.walk_held.is_none() {
                        view.walk_held = Some(view.player.presentation().walking);
                        view.player.send(Command::Walking(true));
                    }
                    cx.stop_propagation();
                }
            }))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, _, cx| {
                if event.keystroke.key == cx.global::<Desktop>().preferences.walk_key {
                    view.release_walk();
                }
            }))
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
            .child(theme::title(title, window, cx))
            .child(
                div()
                    .id("tool-content")
                    .w_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .text_sm()
                    .when(self.kind == Kind::Motion, |root| root.overflow_y_scroll())
                    .child(content),
            )
    }
}
