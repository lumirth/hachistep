use crate::{
    commands::{Intent, Invoke},
    player::{FileCommand, PlayerView},
    settings, Desktop,
};
use gpui_kit::{
    component::{
        button::Button,
        checkbox::Checkbox,
        form::{Field, Form},
        input::{Input, InputState},
        ActiveTheme,
    },
    prelude::*,
    *,
};
use hs_runtime::{Command, Player};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Motion,
    Diagnostics,
    Link,
    Recent,
    Tcp(bool),
}

pub(crate) fn open(
    player: Player,
    owner: WeakEntity<PlayerView>,
    kind: Kind,
    _parent: &mut Window,
    cx: &mut App,
) {
    cx.defer(move |cx| open_tool(player, owner, kind, cx));
}

fn open_tool(player: Player, owner: WeakEntity<PlayerView>, kind: Kind, cx: &mut App) {
    let name = match kind {
        Kind::Motion => "Walking Simulation",
        Kind::Diagnostics => "Diagnostics",
        Kind::Link => "Local Link",
        Kind::Recent => "Open Recent",
        Kind::Tcp(false) => "Connect over TCP",
        Kind::Tcp(true) => "Listen over TCP",
    };
    let filename = player
        .presentation()
        .eeprom
        .as_deref()
        .map(settings::filename)
        .unwrap_or_else(|| "hachistep".into());
    let height = 300.;
    if let Err(error) = gpui_kit::open_window(
        crate::options(&format!("{name} — {filename}"), 460., height),
        cx,
        |window, cx| cx.new(|cx| Tool::new(player, owner, kind, window, cx)),
    ) {
        eprintln!("Could not open {name}: {error:#}");
    }
}

struct Tool {
    player: Player,
    owner: WeakEntity<PlayerView>,
    kind: Kind,
    cadence: Entity<InputState>,
    address: Entity<InputState>,
    remote: bool,
    error: Option<String>,
    focus: FocusHandle,
}

impl Tool {
    fn new(
        player: Player,
        owner: WeakEntity<PlayerView>,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let cadence = cx.new(|cx| {
            InputState::new(window, cx).default_value(player.presentation().cadence.to_string())
        });
        let address = cx.new(|cx| {
            InputState::new(window, cx).default_value(if matches!(kind, Kind::Tcp(true)) {
                "127.0.0.1:8083"
            } else {
                "127.0.0.1:8081"
            })
        });
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            if cx
                .update(|window, cx| {
                    view.update(cx, |view, cx| {
                        if view.player.presentation().closed {
                            window.remove_window();
                        } else {
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
            address,
            remote: false,
            error: None,
            focus,
        }
    }
    fn invoke(&self, intent: Intent, _window: &mut Window, cx: &mut App) {
        let handle = cx
            .global::<Desktop>()
            .windows
            .iter()
            .find(|w| w.view.entity_id() == self.owner.entity_id())
            .map(|w| w.handle);
        if let Some(handle) = handle {
            let _ = handle.update(cx, |_, window, cx| {
                self.owner
                    .update(cx, |owner, cx| owner.execute(intent, window, cx))
            });
        }
    }
    fn walking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.cadence.read(cx).value().parse::<u16>() {
            Ok(cadence) if (30..=240).contains(&cadence) => {
                self.player.send(Command::Cadence(cadence));
                self.invoke(Intent::Walking, window, cx);
                self.error = None;
            }
            _ => self.error = Some("Enter a cadence from 30 to 240 steps/min.".into()),
        }
        cx.notify();
    }
    fn motion(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        Form::new()
            .child(
                Field::new()
                    .label("Cadence · steps/min")
                    .child(Input::new(&self.cadence)),
            )
            .child(Field::new().label("Motion").child(if frame.walking {
                "Walking"
            } else {
                "Still"
            }))
            .footer(
                div().flex().gap_2().child(
                    Button::new("walking")
                        .label(if frame.walking {
                            "Stop Walking"
                        } else {
                            "Start Walking"
                        })
                        .on_click(cx.listener(|view, _, window, cx| view.walking(window, cx))),
                ),
            )
            .into_any_element()
    }
    fn diagnostics(&self, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        let mut form = Form::new()
            .child(
                Field::new()
                    .label("Application")
                    .child(env!("CARGO_PKG_VERSION")),
            )
            .child(
                Field::new()
                    .label("Core revision")
                    .child(hs_runtime::CORE_REVISION),
            )
            .child(
                Field::new().label("Firmware SHA-256").child(
                    div()
                        .font_family("monospace")
                        .text_xs()
                        .child(frame.firmware.clone()),
                ),
            )
            .child(
                Field::new().label("EEPROM").child(
                    frame
                        .eeprom
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            )
            .child(
                Field::new()
                    .label("Execution")
                    .child(format!("{:.2}× · {:?}", frame.speed, frame.time)),
            )
            .child(
                Field::new()
                    .label("Infrared")
                    .child(frame.connection_name()),
            );
        if let Some(error) = frame.audio_error {
            form =
                form.child(Field::new().label("Audio").child(error).child(
                    Button::new("retry-audio").label("Retry Audio").on_click(
                        cx.listener(|view, _, _, _| view.player.send(Command::RetryAudio)),
                    ),
                ));
        }
        form.footer(
            Button::new("export")
                .label("Export Diagnostics…")
                .on_click(cx.listener(|view, _, window, cx| {
                    let _ = view.owner.update(cx, |owner, cx| {
                        owner.choose(FileCommand::Diagnostics, window, cx)
                    });
                })),
        )
        .into_any_element()
    }
    fn link(&self, cx: &mut Context<Self>) -> AnyElement {
        let peers: Vec<_> = cx
            .global::<Desktop>()
            .windows
            .iter()
            .filter(|w| w.view.entity_id() != self.owner.entity_id())
            .filter_map(|w| w.view.upgrade())
            .filter_map(|view| {
                let view = view.read(cx);
                let frame = view.presentation();
                (frame.ready
                    && frame.peer.is_none()
                    && frame.connection == hs_runtime::Connection::Disconnected)
                    .then(|| (view.player().id(), view.filename()))
            })
            .collect();
        if peers.is_empty() {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child("Open another player to connect.")
                .child(
                    Button::new("new-window")
                        .label("New Window")
                        .on_click(cx.listener(|_, _, _, cx| crate::new_window(cx, None))),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .children(peers.into_iter().map(|(id, name)| {
                    Button::new(("peer", id)).label(name).on_click(cx.listener(
                        move |view, _, window, cx| {
                            view.invoke(Intent::Connect(id), window, cx);
                            window.remove_window();
                        },
                    ))
                }))
                .into_any_element()
        }
    }
    fn recent(&self, cx: &mut Context<Self>) -> AnyElement {
        let recent = cx.global::<Desktop>().preferences.recent.clone();
        if recent.is_empty() {
            return div().child("No recent files.").into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(recent.into_iter().enumerate().map(|(ix, pair)| {
                Button::new(("recent", ix))
                    .label(settings::filename(&pair.eeprom))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.invoke(Intent::Recent(pair.clone()), window, cx);
                        window.remove_window();
                    }))
            }))
            .into_any_element()
    }
    fn tcp(&self, listen: bool, cx: &mut Context<Self>) -> AnyElement {
        let frame = self.player.presentation();
        Form::new()
            .child(
                Field::new()
                    .label(if listen {
                        "Listen address"
                    } else {
                        "Peer address"
                    })
                    .child(Input::new(&self.address)),
            )
            .child(
                Field::new().label_indent(false).child(
                    Checkbox::new("remote")
                        .label("Allow remote connections")
                        .checked(self.remote)
                        .on_change(cx.listener(|view, value, _, cx| {
                            view.remote = *value;
                            cx.notify();
                        })),
                ),
            )
            .child(
                Field::new()
                    .label("Connection")
                    .child(frame.connection_name()),
            )
            .child(
                Field::new().label_indent(false).child(
                    Checkbox::new("mute-session")
                        .label("Mute this player")
                        .checked(frame.muted)
                        .on_change(
                            cx.listener(|view, value, _, _| {
                                view.player.send(Command::Mute(*value))
                            }),
                        ),
                ),
            )
            .footer(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("connect")
                            .label(if listen { "Start Listening" } else { "Connect" })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let address = view.address.read(cx).value().to_string();
                                view.player.send(Command::Tcp {
                                    address,
                                    listen,
                                    allow_remote: view.remote,
                                });
                            })),
                    )
                    .child(Button::new("disconnect").label("Disconnect").on_click(
                        cx.listener(|view, _, _, _| view.player.send(Command::Disconnect)),
                    )),
            )
            .into_any_element()
    }
}

impl Render for Tool {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.kind {
            Kind::Motion => self.motion(cx),
            Kind::Diagnostics => self.diagnostics(cx),
            Kind::Link => self.link(cx),
            Kind::Recent => self.recent(cx),
            Kind::Tcp(listen) => self.tcp(listen, cx),
        };
        div()
            .id("tool")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|view, action: &Invoke, window, cx| {
                if action.intent == Intent::Close {
                    window.remove_window();
                } else {
                    view.invoke(action.intent.clone(), window, cx);
                }
            }))
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p_4()
                    .gap_3()
                    .child(content)
                    .when_some(self.error.clone(), |root, error| {
                        root.child(div().text_color(cx.theme().danger).child(error))
                    }),
            )
    }
}
