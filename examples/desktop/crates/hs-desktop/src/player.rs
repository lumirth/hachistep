use crate::{
    commands::{self, Intent, Invoke},
    lcd,
    settings::{self, Recent},
    Desktop,
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        menu::AppMenuBar,
        ActiveTheme, WindowExt,
    },
    prelude::*,
    *,
};
use hs_runtime::{Buttons, Command, Player, Presentation};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Copy)]
pub(crate) enum FileCommand {
    Firmware,
    Eeprom,
    NewEeprom,
    SaveEepromAs,
    SaveState,
    LoadState,
    Screenshot,
    Diagnostics,
}

pub(crate) struct PlayerView {
    player: Player,
    presentation: Presentation,
    firmware: Option<PathBuf>,
    pending: Option<(Player, Recent)>,
    focus: FocusHandle,
    buttons: Buttons,
    scale: u8,
    raw: bool,
    status: bool,
    error: Option<String>,
    last_error: Option<String>,
    menu_bar: Entity<AppMenuBar>,
    menu_state: String,
    closing: bool,
    _subscriptions: Vec<Subscription>,
}

impl PlayerView {
    pub fn player(&self) -> &Player {
        &self.player
    }
    pub fn filename(&self) -> String {
        self.presentation
            .eeprom
            .as_deref()
            .map(settings::filename)
            .unwrap_or_else(|| "hachistep".into())
    }
    pub fn presentation(&self) -> &Presentation {
        &self.presentation
    }
    pub fn new(player: Player, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let activated = cx.observe_window_activation(window, |view, window, cx| {
            view.release();
            view.player.send(Command::Focus(
                window.is_window_active() && view.focus.is_focused(window),
            ));
            if window.is_window_active() {
                view.install_menu(cx);
            }
        });
        let lost = cx.on_blur(&focus, window, |view, _, _| {
            view.release();
            view.player.send(Command::Focus(false));
        });
        let focused = cx.on_focus(&focus, window, |view, _, _| {
            view.player.send(Command::Focus(true))
        });
        let release = cx.on_release(|view, _| view.player.send(Command::Close));
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |view, _| {
                view.player.send(Command::Close);
                view.closing = true;
            })
            .ok();
            false
        });
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            if cx
                .update(|window, cx| view.update(cx, |view, cx| view.refresh(window, cx)))
                .is_err()
            {
                break;
            }
        })
        .detach();
        Self {
            player,
            presentation: Presentation::default(),
            firmware: None,
            pending: None,
            focus,
            buttons: Buttons::RELEASED,
            scale: 4,
            raw: false,
            status: false,
            error: None,
            last_error: None,
            menu_bar: AppMenuBar::new(cx),
            menu_state: String::new(),
            closing: false,
            _subscriptions: vec![activated, lost, focused, release],
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((player, pair)) = &self.pending {
            let view = player.presentation();
            if view.ready {
                let player = player.clone();
                let pair = pair.clone();
                self.player.send(Command::Close);
                self.player = player;
                self.pending = None;
                self.firmware = Some(pair.firmware.clone());
                let desktop = cx.global_mut::<Desktop>();
                desktop.preferences.remember(pair);
                crate::save_preferences(cx);
                self.player
                    .send(Command::Mute(cx.global::<Desktop>().preferences.mute));
                self.player
                    .send(Command::Speed(cx.global::<Desktop>().preferences.speed));
                self.player
                    .send(Command::TurboRate(cx.global::<Desktop>().preferences.turbo));
                let mapping = cx
                    .global::<Desktop>()
                    .preferences
                    .gamepad
                    .map(|ix| settings::GAMEPAD[ix.min(settings::GAMEPAD.len() - 1)].0);
                self.player.send(Command::GamepadMapping(mapping));
                self.focus.focus(window, cx);
                self.player.send(Command::Focus(window.is_window_active()));
                self.error = None;
            } else if let Some(error) = view.error {
                player.send(Command::Close);
                self.pending = None;
                self.error = Some(error);
            }
        }
        let presentation = self.player.presentation();
        if presentation.closed && self.closing {
            let entity_id = cx.entity_id();
            let desktop = cx.global_mut::<Desktop>();
            desktop
                .windows
                .retain(|w| w.view.upgrade().is_some_and(|v| v.entity_id() != entity_id));
            let quitting = desktop.quitting && desktop.windows.is_empty();
            window.remove_window();
            if quitting {
                cx.quit();
            }
            return;
        }
        let changed = self.presentation.pixels != presentation.pixels
            || self.presentation.ready != presentation.ready
            || self.presentation.paused != presentation.paused
            || self.presentation.walking != presentation.walking
            || self.presentation.eeprom != presentation.eeprom
            || self.presentation.peer != presentation.peer
            || self.presentation.connection != presentation.connection
            || self.presentation.eeprom_error != presentation.eeprom_error
            || self.presentation.contrast != presentation.contrast
            || self.presentation.speed_percent != presentation.speed_percent
            || self.presentation.turbo != presentation.turbo
            || self.presentation.muted != presentation.muted
            || self.presentation.can_undo != presentation.can_undo
            || self.presentation.slots != presentation.slots
            || self.status;
        self.presentation = presentation;
        let mut title = self
            .presentation
            .eeprom
            .as_ref()
            .map(|_| format!("hachistep — {}", self.filename()))
            .unwrap_or_else(|| "hachistep".into());
        if self.presentation.paused {
            title.push_str(" — Paused");
        }
        if self.presentation.speed_percent != 100 {
            title.push_str(&format!(" — {}%", self.presentation.speed_percent));
        }
        if self.presentation.turbo {
            title.push_str(" — Fast-forward");
        }
        if self.presentation.walking {
            title.push_str(" — Walking");
        }
        if self.presentation.peer.is_some()
            || self.presentation.connection == hs_runtime::Connection::Connected
        {
            title.push_str(" — Linked");
        }
        if self.presentation.eeprom_error.is_some() {
            title.push_str(" — EEPROM error");
        }
        window.set_window_title(&title);
        if let Some(error) = self.presentation.error.clone().or(self.error.clone()) {
            if self.last_error.as_ref() != Some(&error) {
                self.last_error = Some(error.clone());
                self.closing = false;
                self.release();
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog.title("hachistep").child(error.clone())
                });
            }
        } else {
            self.last_error = None;
        }
        if changed {
            cx.notify();
        }
        if window.is_window_active() {
            let state = format!(
                "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{:?}:{}:{}",
                self.presentation.ready,
                self.presentation.paused,
                self.presentation.walking,
                self.raw,
                self.status,
                self.scale,
                self.presentation.speed_percent,
                self.presentation.muted,
                cx.global::<Desktop>().preferences.recent.len(),
                self.presentation.can_undo,
                self.presentation.slots,
                self.selected_slot(cx),
                self.presentation.turbo
            );
            if state != self.menu_state {
                self.menu_state = state;
                self.install_menu(cx);
            }
        }
    }

    fn selected_slot(&self, cx: &App) -> u8 {
        self.presentation
            .eeprom
            .as_ref()
            .and_then(|path| cx.global::<Desktop>().preferences.slots.get(path))
            .copied()
            .unwrap_or(1)
    }

    fn select_slot(&self, slot: u8, cx: &mut App) {
        if let Some(path) = &self.presentation.eeprom {
            cx.global_mut::<Desktop>()
                .preferences
                .slots
                .insert(path.clone(), slot.clamp(1, 10));
            crate::save_preferences(cx);
            self.install_menu(cx);
        }
    }

    fn slot(&self, slot: u8, load: bool, cx: &mut App) {
        if let Some(eeprom) = &self.presentation.eeprom {
            self.select_slot(slot, cx);
            let path = hs_runtime::slot_path(eeprom, slot);
            self.player.send(if load {
                Command::LoadState(path)
            } else {
                Command::SaveState(path)
            });
        }
    }

    fn install_menu(&self, cx: &mut App) {
        cx.set_menus(commands::menus(
            &self.presentation,
            self.raw,
            self.status,
            self.scale,
            &cx.global::<Desktop>().preferences.recent,
            self.selected_slot(cx),
        ));
    }
    pub fn begin_close(&mut self) {
        self.closing = true;
        self.player.send(Command::Close);
    }
    pub fn release(&mut self) {
        self.buttons = Buttons::RELEASED;
        self.player.send(Command::Turbo(false));
        self.player.send(Command::Release);
    }

    pub fn open_pair(&mut self, pair: Recent, create: bool, cx: &mut Context<Self>) {
        self.release();
        let canonical = pair.eeprom.canonicalize().ok();
        let existing: Vec<_> = cx
            .global::<Desktop>()
            .windows
            .iter()
            .map(|w| (w.handle, w.view.clone()))
            .collect();
        for (handle, view) in existing {
            if let Some(view) = view.upgrade() {
                if view.entity_id() != cx.entity_id()
                    && canonical.is_some()
                    && view.read(cx).presentation.eeprom.as_ref() == canonical.as_ref()
                {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                    return;
                }
            }
        }
        if self.presentation.eeprom.as_ref() == canonical.as_ref() && canonical.is_some() {
            return;
        }
        if self.presentation.ready {
            crate::new_window(cx, Some(pair));
            return;
        }
        let candidate = cx.global::<Desktop>().runtime.player();
        candidate.send(Command::Open {
            firmware: pair.firmware.clone(),
            eeprom: pair.eeprom.clone(),
            create,
        });
        self.pending = Some((candidate, pair));
        self.error = None;
        cx.notify();
    }

    pub fn execute(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        self.release();
        match intent {
            Intent::NewWindow => crate::new_window(cx, None),
            Intent::Quit => crate::quit(cx),
            Intent::OpenFirmware => self.choose(FileCommand::Firmware, window, cx),
            Intent::OpenEeprom => self.choose(FileCommand::Eeprom, window, cx),
            Intent::NewEeprom => self.choose(FileCommand::NewEeprom, window, cx),
            Intent::SaveEepromAs => self.choose(FileCommand::SaveEepromAs, window, cx),
            Intent::SaveState => self.choose(FileCommand::SaveState, window, cx),
            Intent::LoadState => self.choose(FileCommand::LoadState, window, cx),
            Intent::SaveSelectedState => self.slot(self.selected_slot(cx), false, cx),
            Intent::LoadSelectedState => self.slot(self.selected_slot(cx), true, cx),
            Intent::SaveSlot(slot) => self.slot(slot, false, cx),
            Intent::LoadSlot(slot) => self.slot(slot, true, cx),
            Intent::SelectSlot(slot) => self.select_slot(slot, cx),
            Intent::UndoLoadState => self.player.send(Command::UndoLoadState),
            Intent::ToggleTurbo => self.player.send(Command::ToggleTurbo),
            Intent::Screenshot => self.choose(FileCommand::Screenshot, window, cx),
            Intent::Close => {
                self.closing = true;
                self.player.send(Command::Close);
            }
            Intent::Pause => self.player.send(Command::Pause),
            Intent::Reset => self.player.send(Command::Reset),
            Intent::Walking => self
                .player
                .send(Command::Walking(!self.presentation.walking)),
            Intent::Mute => self.player.send(Command::Mute(!self.presentation.muted)),
            Intent::Speed(percent) => self.player.send(Command::Speed(percent)),
            Intent::Scale(scale) => {
                self.scale = scale;
                self.fit(window);
            }
            Intent::Fit => self.fit(window),
            Intent::Fullscreen => window.toggle_fullscreen(),
            Intent::Status => {
                self.status = !self.status;
                self.fit(window);
            }
            Intent::Raw => self.raw = !self.raw,
            Intent::Connect(peer) => self.player.send(Command::Link(peer)),
            Intent::Disconnect => self.player.send(Command::Disconnect),
            Intent::Recent(pair) => self.open_pair(pair, false, cx),
            Intent::Recents => crate::tools::open(
                self.player.clone(),
                cx.weak_entity(),
                crate::tools::Kind::Recent,
                window,
                cx,
            ),
            Intent::LocalLink => crate::tools::open(
                self.player.clone(),
                cx.weak_entity(),
                crate::tools::Kind::Link,
                window,
                cx,
            ),
            Intent::TcpConnect => crate::tools::open(
                self.player.clone(),
                cx.weak_entity(),
                crate::tools::Kind::Tcp(false),
                window,
                cx,
            ),
            Intent::TcpListen => crate::tools::open(
                self.player.clone(),
                cx.weak_entity(),
                crate::tools::Kind::Tcp(true),
                window,
                cx,
            ),
            Intent::Motion => crate::tools::open(
                self.player.clone(),
                cx.weak_entity(),
                crate::tools::Kind::Motion,
                window,
                cx,
            ),
            Intent::Settings => crate::preferences::open(cx),
            Intent::Diagnostics => crate::tools::open(
                self.player.clone(),
                cx.weak_entity(),
                crate::tools::Kind::Diagnostics,
                window,
                cx,
            ),
            Intent::Controls => {
                let [left, center, right] = cx.global::<Desktop>().preferences.keys.clone();
                let turbo = cx.global::<Desktop>().preferences.turbo_key.to_uppercase();
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog
                        .title("Controls")
                        .child(format!(
                            "{} · Left    {} · Center    {} · Right",
                            left.to_uppercase(),
                            center.to_uppercase(),
                            right.to_uppercase()
                        ))
                        .child(format!("{turbo} · Hold fast-forward"))
                });
            }
            Intent::About => window.open_dialog(cx, |dialog, _, _| {
                dialog
                    .title("hachistep")
                    .child(img("icon.png").size_16())
                    .child(format!(
                        "Pokéwalker emulator · {}",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .child("MIT license")
            }),
            Intent::Documentation => cx.open_url("https://github.com/lumirth/hachistep#readme"),
            Intent::Report => cx.open_url("https://github.com/lumirth/hachistep/issues/new"),
        }
        cx.notify();
    }

    fn fit(&self, window: &mut Window) {
        let dpi = window.scale_factor();
        let physical = (f32::from(self.scale) * dpi).round().max(1.);
        let menu = if cfg!(target_os = "macos") {
            0.
        } else {
            f32::from(window.rem_size()) * 2.
        };
        let status = if self.status {
            f32::from(window.rem_size()) * 1.5
        } else {
            0.
        };
        window.resize(size(
            px(96. * physical / dpi),
            px(64. * physical / dpi + menu + status),
        ));
    }

    pub fn choose(&mut self, operation: FileCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.release();
        let open = matches!(
            operation,
            FileCommand::Firmware | FileCommand::Eeprom | FileCommand::LoadState
        );
        if open {
            let prompt = match operation {
                FileCommand::Firmware => "Open Firmware",
                FileCommand::Eeprom => "Open EEPROM",
                _ => "Load State",
            };
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some(prompt.into()),
            });
            cx.spawn_in(window, async move |view, cx| match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        let _ = cx.update(|window, cx| {
                            view.update(cx, |view, cx| view.selected(operation, path, window, cx))
                        });
                    }
                }
                Ok(Err(error)) => {
                    let _ = view.update(cx, |view, cx| {
                        view.error = Some(error.to_string());
                        cx.notify();
                    });
                }
                _ => (),
            })
            .detach();
        } else {
            let name = match operation {
                FileCommand::NewEeprom | FileCommand::SaveEepromAs => "walker.bin",
                FileCommand::SaveState => "walker.hstepstate",
                FileCommand::Screenshot => "screen.png",
                FileCommand::Diagnostics => "hachistep-diagnostics.json",
                _ => unreachable!(),
            };
            let directory = self
                .presentation
                .eeprom
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let receiver = cx.prompt_for_new_path(&directory, Some(name));
            cx.spawn_in(window, async move |view, cx| match receiver.await {
                Ok(Ok(Some(path))) => {
                    let _ = cx.update(|window, cx| {
                        view.update(cx, |view, cx| view.selected(operation, path, window, cx))
                    });
                }
                Ok(Err(error)) => {
                    let _ = view.update(cx, |view, cx| {
                        view.error = Some(error.to_string());
                        cx.notify();
                    });
                }
                _ => (),
            })
            .detach();
        }
    }
    fn selected(
        &mut self,
        operation: FileCommand,
        path: PathBuf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match operation {
            FileCommand::Firmware => {
                self.firmware = Some(path);
            }
            FileCommand::Eeprom | FileCommand::NewEeprom => {
                if let Some(firmware) = self.firmware.clone() {
                    self.open_pair(
                        Recent {
                            firmware,
                            eeprom: path,
                        },
                        matches!(operation, FileCommand::NewEeprom),
                        cx,
                    );
                } else {
                    self.error = Some("Open firmware first.".into());
                }
            }
            FileCommand::SaveEepromAs => self.player.send(Command::SaveEepromAs(path)),
            FileCommand::SaveState => self.player.send(Command::SaveState(path)),
            FileCommand::LoadState => self.player.send(Command::LoadState(path)),
            FileCommand::Screenshot => match lcd::screenshot(
                &self.presentation.pixels,
                self.presentation.contrast,
                self.raw,
            ) {
                Ok(bytes) => self.player.send(Command::WriteFile { path, bytes }),
                Err(error) => self.error = Some(error.to_string()),
            },
            FileCommand::Diagnostics => {
                let bytes = serde_json::to_vec_pretty(&serde_json::json!({ "application":env!("CARGO_PKG_VERSION"),"core":hs_runtime::CORE_REVISION,"speed":self.presentation.speed,"connection":self.presentation.connection_name(),"scale":self.scale,"paused":self.presentation.paused })).unwrap();
                self.player.send(Command::WriteFile { path, bytes });
            }
        }
        cx.notify();
    }
    fn key(&mut self, key: &str, pressed: bool, window: &Window, cx: &App) {
        if !self.focus.is_focused(window) || !window.is_window_active() {
            return;
        }
        let keys = &cx.global::<Desktop>().preferences.keys;
        if key == keys[0] {
            self.buttons.left = pressed;
        }
        if key == keys[1] {
            self.buttons.center = pressed;
        }
        if key == keys[2] {
            self.buttons.right = pressed;
        }
        if key == cx.global::<Desktop>().preferences.turbo_key {
            self.player.send(Command::Turbo(pressed));
        }
        self.player.send(Command::Buttons(self.buttons));
    }
    fn empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6();
        if self.pending.is_some() {
            content = content.child("Opening…");
        } else if let Some(firmware) = &self.firmware {
            content = content
                .child(settings::filename(firmware))
                .child(Button::new("open-save").label("Open EEPROM…").on_click(
                    cx.listener(|view, _, window, cx| view.execute(Intent::OpenEeprom, window, cx)),
                ))
                .child(
                    Button::new("new-save")
                        .ghost()
                        .label("New EEPROM…")
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.execute(Intent::NewEeprom, window, cx)
                        })),
                );
        } else {
            content = content
                .child(
                    Button::new("open-firmware")
                        .label("Open Firmware…")
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.execute(Intent::OpenFirmware, window, cx)
                        })),
                )
                .child(
                    Button::new("recent")
                        .ghost()
                        .label("Open Recent…")
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.execute(Intent::Recents, window, cx)
                        })),
                );
        }
        content.into_any_element()
    }
}

impl Render for PlayerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.presentation.ready;
        let content = if loaded {
            lcd::display(
                self.presentation.pixels.clone(),
                self.presentation.contrast,
                self.raw,
            )
            .into_any_element()
        } else {
            self.empty(cx)
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|view, action: &Invoke, window, cx| {
                view.execute(action.intent.clone(), window, cx)
            }))
            .when(!cfg!(target_os = "macos"), |root| {
                root.child(div().h_8().child(self.menu_bar.clone()).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, _, _| view.release()),
                ))
            })
            .child(
                div()
                    .id("player")
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus)
                    .key_context("Player")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, window, cx| view.focus.focus(window, cx)),
                    )
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                        if !event.is_held
                            && !event.keystroke.modifiers.platform
                            && !event.keystroke.modifiers.control
                            && !event.keystroke.modifiers.alt
                        {
                            view.key(&event.keystroke.key, true, window, cx);
                        }
                    }))
                    .on_key_up(cx.listener(|view, event: &KeyUpEvent, window, cx| {
                        view.key(&event.keystroke.key, false, window, cx)
                    }))
                    .child(content),
            )
            .when_some(self.presentation.eeprom_error.clone(), |root, error| {
                root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .bg(cx.theme().warning)
                        .text_color(cx.theme().foreground)
                        .child(error)
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(Button::new("retry-save").label("Retry").on_click(
                                    cx.listener(|view, _, _, _| {
                                        view.player.send(Command::RetrySave)
                                    }),
                                ))
                                .child(Button::new("save-as").label("Save EEPROM As…").on_click(
                                    cx.listener(|view, _, window, cx| {
                                        view.choose(FileCommand::SaveEepromAs, window, cx)
                                    }),
                                ))
                                .child(Button::new("save-state").label("Save State…").on_click(
                                    cx.listener(|view, _, window, cx| {
                                        view.choose(FileCommand::SaveState, window, cx)
                                    }),
                                )),
                        ),
                )
            })
            .when(self.status && loaded, |root| {
                root.child(
                    div()
                        .h_6()
                        .px_2()
                        .text_xs()
                        .flex()
                        .items_center()
                        .child(format!(
                            "{:.2}× · {:?} · {}",
                            self.presentation.speed,
                            self.presentation.time,
                            self.presentation.connection
                        )),
                )
            })
    }
}
