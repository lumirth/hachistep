use crate::{
    commands::{self, Intent},
    lcd,
    settings::{self, Recent},
    theme, Desktop,
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        menu::AppMenuBar,
        status_bar::StatusBar,
        v_flex, ActiveTheme, Sizable, WindowExt,
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
    SaveEepromAs,
    SaveState,
    LoadState,
    Screenshot,
}

#[derive(Clone, Copy)]
enum Closing {
    Open,
    Preparing(bool),
    Prompt(bool),
    Saving(bool),
    Committed,
}
impl Closing {
    fn resume(self) -> bool {
        matches!(
            self,
            Self::Preparing(true) | Self::Prompt(true) | Self::Saving(true)
        )
    }
    fn pending(self) -> bool {
        matches!(self, Self::Preparing(_) | Self::Prompt(_) | Self::Saving(_))
    }
}

pub(crate) struct PlayerView {
    player: Player,
    presentation: Presentation,
    firmware: Option<PathBuf>,
    pending: Option<(Player, PathBuf, Option<PathBuf>)>,
    focus: FocusHandle,
    buttons: Buttons,
    scale: u8,
    raw: bool,
    status: bool,
    pending_status: Option<(bool, Size<Pixels>)>,
    error: Option<String>,
    last_error: Option<String>,
    menu_bar: Entity<AppMenuBar>,
    closing: Closing,
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
            .unwrap_or_else(|| {
                if self.presentation.ready {
                    "Untitled EEPROM".into()
                } else {
                    "hachistep".into()
                }
            })
    }
    pub fn presentation(&self) -> &Presentation {
        &self.presentation
    }
    pub fn new(
        player: Player,
        firmware: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let activated = cx.observe_window_activation(window, |view, window, cx| {
            view.release();
            if window.is_window_active()
                && !window.has_active_dialog(cx)
                && !window.has_active_sheet(cx)
            {
                view.focus.focus(window, cx);
            }
            view.player.send(Command::Focus(
                window.is_window_active() && view.focus.is_focused(window),
            ));
            if window.is_window_active() {
                cx.global_mut::<Desktop>().active_player = Some(cx.weak_entity());
                view.install_menu(cx);
            }
        });
        let appearance = cx.observe_window_appearance(window, |_, _, cx| {
            if cx.global::<Desktop>().preferences.appearance == 0 {
                theme::apply(0, cx);
            }
        });
        let bounds = cx.observe_window_bounds(window, |view, window, cx| {
            if let Some((status, target)) = view.pending_status {
                let actual = window.viewport_size();
                if (f32::from(actual.height - target.height)).abs() < 1. {
                    view.status = status;
                    view.pending_status = None;
                    cx.notify();
                }
            }
            let mut available = window.viewport_size();
            available.height -= theme::player_chrome(window.rem_size(), view.status);
            let scale = lcd::scale(available, window.scale_factor()) / window.scale_factor();
            view.scale = if scale.fract() == 0. { scale as u8 } else { 0 };
        });
        let lost = cx.on_blur(&focus, window, |view, _, _| {
            view.release();
            view.player.send(Command::Focus(false));
        });
        let focused = cx.on_focus(&focus, window, |view, _, _| {
            view.player.send(Command::Focus(true))
        });
        let release = cx.on_release(|view, _| view.begin_close());
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |view, cx| {
                view.request_close(window, cx);
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
            firmware,
            pending: None,
            focus,
            buttons: Buttons::RELEASED,
            scale: 4,
            raw: false,
            status: false,
            pending_status: None,
            error: None,
            last_error: None,
            menu_bar: AppMenuBar::new(cx),
            closing: Closing::Open,
            _subscriptions: vec![activated, lost, focused, release, appearance, bounds],
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((player, firmware, eeprom)) = &self.pending {
            let view = player.presentation();
            if view.ready {
                let player = player.clone();
                let firmware = firmware.clone();
                let eeprom = eeprom.clone();
                self.player.send(Command::Close);
                self.player = player;
                self.pending = None;
                self.firmware = Some(firmware.clone());
                if let Some(eeprom) = eeprom {
                    cx.global_mut::<Desktop>()
                        .preferences
                        .remember(Recent { firmware, eeprom });
                    crate::save_preferences(cx);
                }
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
        if presentation.closed && !matches!(self.closing, Closing::Open) {
            let entity_id = cx.entity_id();
            let desktop = cx.global_mut::<Desktop>();
            desktop
                .windows
                .retain(|w| w.view.upgrade().is_some_and(|v| v.entity_id() != entity_id));
            desktop
                .tool_bounds
                .retain(|(id, _), _| *id != self.player.id());
            desktop.tcp_options.remove(&self.player.id());
            let quitting = desktop.quitting && desktop.windows.is_empty();
            window.remove_window();
            if quitting {
                cx.quit();
            } else if desktop.quitting {
                cx.defer(crate::close_players);
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
            || self.status
            || self.presentation.saving != presentation.saving
            || self.presentation.modified != presentation.modified;
        if self.presentation.eeprom != presentation.eeprom {
            if let (Some(firmware), Some(eeprom)) = (&self.firmware, &presentation.eeprom) {
                cx.global_mut::<Desktop>().preferences.remember(Recent {
                    firmware: firmware.clone(),
                    eeprom: eeprom.clone(),
                });
                crate::save_preferences(cx);
            }
        }
        self.presentation = presentation;
        window.set_window_edited(self.presentation.modified);
        if self.closing.pending()
            && self.presentation.closing
            && self.presentation.paused
            && !self.presentation.saving
        {
            if !self.presentation.modified {
                self.finish_close();
            } else if let Closing::Preparing(resume) = self.closing {
                self.closing = Closing::Prompt(resume);
                let receiver = window.prompt(
                    PromptLevel::Warning,
                    &format!("Save changes to \"{}\"?", self.filename()),
                    None,
                    &["Save", "Cancel", "Don't Save"],
                    cx,
                );
                cx.spawn_in(window, async move |view, cx| {
                    let answer = receiver.await.unwrap_or(1);
                    let _ = cx.update(|window, cx| {
                        view.update(cx, |view, cx| match answer {
                            0 => {
                                view.closing = Closing::Saving(view.closing.resume());
                                view.save_eeprom(window, cx);
                            }
                            2 => view.finish_close(),
                            _ => view.cancel_close(cx),
                        })
                    });
                })
                .detach();
            }
        }
        let mut title = if self.presentation.ready {
            format!("hachistep · {}", self.filename())
        } else {
            "hachistep".into()
        };
        if self.presentation.paused {
            title.push_str(" · Paused");
        }
        if self.presentation.speed_percent != 100 {
            title.push_str(&format!(" · {}%", self.presentation.speed_percent));
        }
        if self.presentation.turbo {
            title.push_str(" · Fast-forward");
        }
        if self.presentation.walking {
            title.push_str(" · Walking");
        }
        if self.presentation.peer.is_some()
            || self.presentation.connection == hs_runtime::Connection::Connected
        {
            title.push_str(" · Linked");
        }
        if self.presentation.eeprom_error.is_some() {
            title.push_str(" · EEPROM error");
        }
        window.set_window_title(&title);
        if let Some(error) = self
            .presentation
            .eeprom_error
            .clone()
            .or(self.presentation.error.clone())
            .or(self.error.clone())
        {
            if self.last_error.as_ref() != Some(&error) {
                self.last_error = Some(error.clone());
                if self.closing.pending() {
                    self.cancel_close(cx);
                }
                self.release();
                let eeprom = self.presentation.eeprom_error.is_some();
                let owner = cx.weak_entity();
                window.open_dialog(cx, move |dialog, _, _| {
                    let dialog = dialog
                        .title(if eeprom {
                            "Could not save EEPROM"
                        } else {
                            "hachistep"
                        })
                        .child(error.clone());
                    if !eeprom {
                        return dialog;
                    }
                    let save = owner.clone();
                    let state = owner.clone();
                    let retry = owner.clone();
                    dialog.footer(
                        div()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .child(Button::new("retry-eeprom").label("Retry").on_click(
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = retry.update(cx, |view, _| {
                                        view.player.send(Command::SaveEeprom)
                                    });
                                },
                            ))
                            .child(
                                Button::new("recover-eeprom")
                                    .label("Save EEPROM As…")
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let _ = save.update(cx, |view, cx| {
                                            view.choose(FileCommand::SaveEepromAs, window, cx)
                                        });
                                    }),
                            )
                            .child(Button::new("recover-state").label("Save State…").on_click(
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = state.update(cx, |view, cx| {
                                        view.choose(FileCommand::SaveState, window, cx)
                                    });
                                },
                            )),
                    )
                });
            }
        } else {
            self.last_error = None;
        }
        if changed {
            cx.notify();
        }
        if window.is_window_active() {
            self.install_menu(cx);
        }
    }

    fn menu(&self, cx: &App) -> commands::MenuState {
        commands::MenuState::new(
            &self.presentation,
            self.raw,
            self.status,
            self.scale,
            &cx.global::<Desktop>().preferences,
            self.selected_slot(cx),
        )
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

    pub(crate) fn install_menu(&self, cx: &mut App) {
        self.menu(cx).install(cx);
    }
    pub fn begin_close(&mut self) {
        if let Some((pending, _, _)) = self.pending.take() {
            pending.send(Command::Close);
        }
        self.closing = Closing::Committed;
        self.player.send(Command::Close);
    }
    pub fn release(&mut self) {
        self.buttons = Buttons::RELEASED;
        self.player.send(Command::Turbo(false));
        self.player.send(Command::Release);
    }

    pub fn request_close(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if !matches!(self.closing, Closing::Open) {
            return;
        }
        self.closing = Closing::Preparing(!self.presentation.paused);
        self.release();
        self.player.send(Command::PrepareClose);
    }
    fn cancel_close(&mut self, cx: &mut App) {
        let resume = self.closing.resume() && self.presentation.paused;
        self.closing = Closing::Open;
        cx.global_mut::<Desktop>().quitting = false;
        self.player.send(Command::CancelClose);
        if resume {
            self.player.send(Command::Pause);
        }
    }
    fn finish_close(&mut self) {
        if self.closing.resume() {
            self.player.send(Command::Pause);
        }
        self.begin_close();
    }
    fn save_eeprom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.presentation.eeprom.is_some() {
            self.player.send(Command::SaveEeprom);
        } else {
            self.choose(FileCommand::SaveEepromAs, window, cx);
        }
    }

    pub fn open_pair(&mut self, pair: Recent, cx: &mut Context<Self>) {
        self.open_image(pair.firmware, Some(pair.eeprom), cx);
    }

    pub fn open_image(
        &mut self,
        firmware: PathBuf,
        eeprom: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.release();
        let canonical = eeprom.as_ref().and_then(|path| path.canonicalize().ok());
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
            crate::new_window(
                cx,
                Some(match eeprom {
                    Some(eeprom) => crate::Launch::Files(Recent { firmware, eeprom }),
                    None => crate::Launch::Blank(firmware),
                }),
            );
            return;
        }
        if let Some((pending, _, _)) = self.pending.take() {
            pending.send(Command::Close);
        }
        let candidate = cx.global::<Desktop>().runtime.player();
        candidate.send(Command::Open {
            firmware: firmware.clone(),
            eeprom: eeprom.clone(),
        });
        self.pending = Some((candidate, firmware, eeprom));
        self.error = None;
        cx.notify();
    }

    pub fn execute(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<Desktop>().disabled_commands.contains(&intent) {
            return;
        }
        self.release();
        match intent {
            Intent::NewWindow => crate::new_window(cx, None),
            Intent::Quit => crate::quit(cx),
            Intent::OpenFirmware => self.choose(FileCommand::Firmware, window, cx),
            Intent::OpenEeprom => self.choose(FileCommand::Eeprom, window, cx),
            Intent::NewEeprom => {
                if let Some(firmware) = self.firmware.clone() {
                    self.open_image(firmware, None, cx);
                }
            }
            Intent::SaveEeprom => self.save_eeprom(window, cx),
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
            Intent::Close => self.request_close(window, cx),
            Intent::Pause => self.player.send(Command::Pause),
            Intent::Reset => self.player.send(Command::Reset),
            Intent::Mute => self.player.send(Command::Mute(!self.presentation.muted)),
            Intent::Speed(percent) => self.player.send(Command::Speed(percent)),
            Intent::Scale(scale) => {
                self.scale = scale;
                self.fit(window, Some(scale));
            }
            Intent::Fit => self.fit(window, None),
            Intent::Fullscreen => window.toggle_fullscreen(),
            Intent::Status => {
                let (previous, mut target) = self
                    .pending_status
                    .unwrap_or((self.status, window.viewport_size()));
                let status = !previous;
                if window.is_fullscreen() {
                    self.status = status;
                } else {
                    target.height += theme::player_chrome(window.rem_size(), status)
                        - theme::player_chrome(window.rem_size(), previous);
                    self.pending_status = Some((status, target));
                    window.resize(target);
                }
            }
            Intent::Raw => self.raw = !self.raw,
            Intent::RecentEeprom(pair) => self.open_pair(pair, cx),
            Intent::RecentFirmware(path) => self.selected(FileCommand::Firmware, path, window, cx),
            Intent::Recents => {
                commands::native(self.menu(cx).recents()).show(window.mouse_position(), window, cx);
            }
            Intent::Tool(kind) => {
                crate::tools::open(self.player.clone(), cx.weak_entity(), kind, window, cx)
            }
            Intent::Settings => crate::preferences::open(cx),
            Intent::Controls => crate::preferences::open_controls(cx),
            Intent::About => crate::about::open(cx),
            Intent::Documentation => cx.open_url("https://github.com/lumirth/hachistep#readme"),
            Intent::Report => cx.open_url("https://github.com/lumirth/hachistep/issues/new"),
        }
        cx.notify();
    }

    fn fit(&self, window: &mut Window, preset: Option<u8>) {
        if window.is_fullscreen() {
            return;
        }
        let dpi = window.scale_factor();
        let chrome = theme::player_chrome(window.rem_size(), self.status);
        let mut available = window.viewport_size();
        available.height -= chrome;
        let physical = preset.map_or_else(
            || lcd::scale(available, dpi),
            |scale| (f32::from(scale) * dpi).round().max(1.),
        );
        let mut target = lcd::dimensions(physical, dpi);
        target.height += chrome;
        window.resize(target);
    }

    pub fn choose(&mut self, operation: FileCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.release();
        let open = matches!(
            operation,
            FileCommand::Firmware | FileCommand::Eeprom | FileCommand::LoadState
        );
        let directory = self
            .presentation
            .eeprom
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        cx.spawn_in(window, async move |view, cx| {
            let receiver = if open {
                let prompt = match operation {
                    FileCommand::Firmware => "Open Firmware",
                    FileCommand::Eeprom => "Open EEPROM",
                    _ => "Load State",
                };
                let receiver = cx.update(|_, cx| {
                    cx.prompt_for_paths(PathPromptOptions {
                        files: true,
                        directories: false,
                        multiple: false,
                        prompt: Some(prompt.into()),
                    })
                });
                match receiver {
                    Ok(receiver) => receiver.await.map(|result| {
                        result.map(|paths| paths.and_then(|paths| paths.into_iter().next()))
                    }),
                    Err(_) => return,
                }
            } else {
                let name = match operation {
                    FileCommand::SaveEepromAs => "walker.bin",
                    FileCommand::SaveState => "walker.hstepstate",
                    FileCommand::Screenshot => "screen.png",
                    _ => unreachable!(),
                };
                let receiver = cx.update(|_, cx| cx.prompt_for_new_path(&directory, Some(name)));
                match receiver {
                    Ok(receiver) => receiver.await,
                    Err(_) => return,
                }
            };
            let _ = cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    match receiver {
                        Ok(Ok(Some(path))) => view.selected(operation, path, window, cx),
                        Ok(Err(error)) => view.error = Some(error.to_string()),
                        _ if view.closing.pending() => view.cancel_close(cx),
                        _ => (),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
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
                cx.global_mut::<Desktop>()
                    .preferences
                    .remember_firmware(path.clone());
                crate::save_preferences(cx);
                if self.presentation.ready {
                    crate::new_window(cx, Some(crate::Launch::Firmware(path)));
                } else {
                    if let Some((pending, _, _)) = self.pending.take() {
                        pending.send(Command::Close);
                    }
                    self.firmware = Some(path);
                }
            }
            FileCommand::Eeprom => {
                if let Some(firmware) = self.firmware.clone() {
                    self.open_pair(
                        Recent {
                            firmware,
                            eeprom: path,
                        },
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
                cx.global::<Desktop>().preferences.palette,
            ) {
                Ok(bytes) => self.player.send(Command::WriteFile { path, bytes }),
                Err(error) => self.error = Some(error.to_string()),
            },
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
        if key == cx.global::<Desktop>().preferences.turbo_key
            && !self.presentation.is_connection_active()
        {
            self.player.send(Command::Turbo(pressed));
        }
        self.player.send(Command::Buttons(self.buttons));
    }
    fn empty(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut content = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6();
        if window.viewport_size().width < px(288.) || window.viewport_size().height < px(192.) {
            content = content.p_2().gap_1();
            if self.pending.is_some() {
                return content.child("Opening…").into_any_element();
            }
            let intent = if self.firmware.is_some() {
                Intent::OpenEeprom
            } else {
                Intent::OpenFirmware
            };
            return content
                .child(Button::new("open").small().label("Open…").on_click(
                    cx.listener(move |view, _, window, cx| {
                        view.execute(intent.clone(), window, cx)
                    }),
                ))
                .when(self.firmware.is_some(), |root| {
                    root.child(Button::new("new").small().ghost().label("New…").on_click(
                        cx.listener(|view, _, window, cx| {
                            view.execute(Intent::NewEeprom, window, cx)
                        }),
                    ))
                })
                .into_any_element();
        }
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
                        .label("Create EEPROM…")
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.execute(Intent::NewEeprom, window, cx)
                        })),
                );
        } else {
            content = content
                .child(img("icon.png").size_12())
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.presentation.ready;
        let content = if loaded {
            lcd::display(
                self.presentation.pixels.clone(),
                self.presentation.contrast,
                self.raw,
                cx.global::<Desktop>().preferences.palette,
            )
            .into_any_element()
        } else {
            self.empty(window, cx)
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|view, action: &Intent, window, cx| {
                view.execute(action.clone(), window, cx)
            }))
            .child(theme::title(
                if loaded {
                    let filename = self.filename();
                    if self.presentation.eeprom_error.is_some() {
                        format!("{filename} · EEPROM error")
                    } else if self.presentation.paused {
                        format!("{filename} · Paused")
                    } else if self.presentation.turbo {
                        format!("{filename} · Fast-forward")
                    } else if self.presentation.speed_percent != 100 {
                        format!("{filename} · {}%", self.presentation.speed_percent)
                    } else {
                        filename
                    }
                } else {
                    "hachistep".into()
                },
                window,
                cx,
            ))
            .when(!cfg!(target_os = "macos"), |root| {
                root.child(
                    div()
                        .h(theme::menu_height(window.rem_size()))
                        .flex_shrink_0()
                        .child(self.menu_bar.clone())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, _| view.release()),
                        ),
                )
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
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|view, event: &MouseDownEvent, window, cx| {
                            view.release();
                            view.focus.focus(window, cx);
                            let items = view.menu(cx).context();
                            commands::native(items).show(event.position, window, cx);
                        }),
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
            .when(self.status, |root| {
                let frame = &self.presentation;
                let state = if !loaded {
                    String::new()
                } else if frame.paused {
                    "Paused".into()
                } else if frame.turbo {
                    "Fast-forward".into()
                } else {
                    format!("{}%", frame.speed_percent)
                };
                let connection = frame
                    .peer
                    .and_then(|id| {
                        cx.global::<Desktop>()
                            .windows
                            .iter()
                            .filter_map(|w| w.view.upgrade())
                            .filter(|v| v.entity_id() != cx.entity_id())
                            .find_map(|view| {
                                let view = view.read(cx);
                                (view.player().id() == id)
                                    .then(|| format!("Linked · {}", view.filename()))
                            })
                    })
                    .unwrap_or_else(|| {
                        if frame.connection == hs_runtime::Connection::Disconnected {
                            String::new()
                        } else {
                            frame.connection_name()
                        }
                    });
                let pace = if loaded
                    && !frame.paused
                    && frame.speed > 0.
                    && (frame.speed * 100. - f64::from(frame.speed_percent)).abs() > 5.
                    && !frame.turbo
                {
                    format!("{:.0}% actual", frame.speed * 100.)
                } else {
                    String::new()
                };
                root.child(
                    StatusBar::new()
                        .h(theme::status_height(window.rem_size()))
                        .flex_shrink_0()
                        .py_0()
                        .left(state)
                        .child(pace)
                        .right(connection),
                )
            })
    }
}
