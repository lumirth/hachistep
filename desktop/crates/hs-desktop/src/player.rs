use crate::{
    commands::{self, Intent},
    lcd,
    settings::{self, Recent},
    theme, Desktop,
};
use gpui_kit::{
    component::{
        button::{Button, ButtonRounded, ButtonVariants},
        h_flex,
        menu::AppMenuBar,
        status_bar::StatusBar,
        v_flex, ActiveTheme, Disableable, Sizable, WindowExt,
    },
    prelude::*,
    *,
};
use hs_runtime::{Buttons, Command, Player, Presentation};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

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
    new_after_firmware: bool,
    requested: Option<Recent>,
    palette_id: Option<String>,
    options: settings::WalkerPreferences,
    walk_held: Option<bool>,
    notice: Option<(String, Instant, bool)>,
    pending: Option<(Player, PathBuf, Option<PathBuf>)>,
    focus: FocusHandle,
    buttons: Buttons,
    scale: u8,
    raw: bool,
    status: bool,
    margin: u16,
    pending_layout: Option<(bool, u16, Size<Pixels>)>,
    error: Option<String>,
    last_error: Option<String>,
    menu_bar: Entity<AppMenuBar>,
    menu_state: Option<commands::MenuState>,
    closing: Closing,
    _subscriptions: Vec<Subscription>,
}

fn status_button(id: &'static str) -> Button {
    Button::new(id)
        .xsmall()
        .ghost()
        .rounded(ButtonRounded::Small)
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
            if let Some((status, margin, target)) = view.pending_layout {
                let actual = window.viewport_size();
                if (f32::from(actual.height - target.height)).abs() < 1.
                    && (f32::from(actual.width - target.width)).abs() < 1.
                {
                    view.status = status;
                    view.margin = margin;
                    view.pending_layout = None;
                    cx.notify();
                }
            }
            let mut available = window.viewport_size();
            available.width -= px(f32::from(view.margin) * 2.);
            available.height -= theme::player_chrome(window.rem_size(), view.status)
                + px(f32::from(view.margin) * 2.);
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
            new_after_firmware: false,
            firmware,
            requested: None,
            palette_id: None,
            options: settings::WalkerPreferences::default(),
            walk_held: None,
            notice: None,
            pending: None,
            focus,
            buttons: Buttons::RELEASED,
            scale: 4,
            raw: false,
            status: false,
            margin: cx.global::<Desktop>().preferences.lcd_margin,
            pending_layout: None,
            error: None,
            last_error: None,
            menu_bar: AppMenuBar::new(cx),
            menu_state: None,
            closing: Closing::Open,
            _subscriptions: vec![activated, lost, focused, release, appearance, bounds],
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let margin = cx.global::<Desktop>().preferences.lcd_margin;
        if self
            .pending_layout
            .map_or(self.margin, |(_, margin, _)| margin)
            != margin
        {
            let status = self
                .pending_layout
                .map_or(self.status, |(status, _, _)| status);
            self.layout(status, margin, window);
        }
        if let Some(pair) = self.requested.take() {
            self.resolve_pair(pair, window, cx);
        }

        if let Some((player, firmware, eeprom)) = &self.pending {
            let view = player.presentation();
            if view.ready
                && (!self.presentation.ready || !self.player.presentation().is_connection_active())
            {
                let player = player.clone();
                let firmware = firmware.clone();
                let eeprom = eeprom.clone();
                self.player.send(Command::Close);
                self.player = player;
                self.pending = None;
                self.firmware = Some(firmware.clone());
                if let Some(eeprom) = eeprom {
                    cx.global_mut::<Desktop>().preferences.remember(Recent {
                        firmware_hash: crate::firmware::identify(&firmware).ok(),
                        firmware,
                        eeprom,
                    });
                    crate::save_preferences(cx);
                }
                let mapping = cx
                    .global::<Desktop>()
                    .preferences
                    .gamepad
                    .map(|ix| settings::GAMEPAD[ix.min(settings::GAMEPAD.len() - 1)].0);
                self.player.send(Command::GamepadMapping(mapping));
                self.focus.focus(window, cx);
                self.player.send(Command::Focus(window.is_window_active()));
                self.error = None;
                self.apply_options(window, cx);
            } else if let Some(error) = view.error {
                player.send(Command::Close);
                self.pending = None;
                self.error = Some(error);
            }
        }
        let presentation = self.player.presentation();
        if presentation.closed && !matches!(self.closing, Closing::Open) {
            if let WindowBounds::Windowed(bounds) = window.window_bounds() {
                cx.global_mut::<Desktop>().preferences.player_bounds = Some([
                    bounds.origin.x.into(),
                    bounds.origin.y.into(),
                    bounds.size.width.into(),
                    (bounds.size.height
                        - if self.status {
                            theme::status_height(window.rem_size())
                        } else {
                            px(0.)
                        })
                    .into(),
                ]);
                crate::save_preferences(cx);
            }
            let entity_id = cx.entity_id();
            let desktop = cx.global_mut::<Desktop>();
            desktop
                .windows
                .retain(|w| w.view.upgrade().is_some_and(|v| v.entity_id() != entity_id));
            desktop
                .tool_bounds
                .retain(|(id, _), _| *id != self.player.id());
            desktop.tcp_options.remove(&self.player.id());
            desktop.melon_ports.remove(&self.player.id());
            let quitting = desktop.quitting && desktop.windows.is_empty();
            let empty = desktop.windows.is_empty();
            if empty {
                desktop.active_player = None;
            }
            window.remove_window();
            if empty {
                crate::install_empty_menu(cx);
            }
            if quitting {
                if !crate::macos::finish_quit(true) {
                    cx.quit();
                }
            } else if cx.global::<Desktop>().quitting {
                cx.defer(crate::close_players);
            }
            return;
        }
        let completion = presentation.completion.as_ref().map(|(id, _)| *id);
        if completion != self.presentation.completion.as_ref().map(|(id, _)| *id) {
            if let Some((_, result)) = &presentation.completion {
                use hs_runtime::Completion;
                let message = match result {
                    Completion::SavedEeprom(path) => format!("Saved {}", settings::filename(path)),
                    Completion::SavedState(path) => {
                        let name = settings::filename(path);
                        if let Some(slot) = name
                            .strip_prefix("slot-")
                            .and_then(|s| s.strip_suffix(".hstepstate"))
                        {
                            format!("Saved slot {slot}")
                        } else {
                            format!("Saved {name}")
                        }
                    }
                    Completion::LoadedState => {
                        let undo = commands::shortcuts()
                            .into_iter()
                            .find(|shortcut| shortcut.intent == Intent::UndoLoadState)
                            .unwrap()
                            .key(&cx.global::<Desktop>().preferences);
                        if undo.is_empty() {
                            "Loaded state".into()
                        } else {
                            format!("Loaded state. Press {} to undo.", settings::key_name(&undo))
                        }
                    }
                    Completion::UndidLoad => "Undid the state load".into(),
                    Completion::Written(path) => format!("Saved {}", settings::filename(path)),
                };
                self.notify(message, false, cx);
            }
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, until, _)| Instant::now() >= *until)
        {
            self.notice = None;
            cx.notify();
        }
        let changed = self.presentation.pixels != presentation.pixels
            || self.presentation.ready != presentation.ready
            || self.presentation.paused != presentation.paused
            || self.presentation.walking != presentation.walking
            || self.presentation.eeprom != presentation.eeprom
            || self.presentation.peer != presentation.peer
            || self.presentation.connection != presentation.connection
            || self.presentation.melon_ds != presentation.melon_ds
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
                    firmware_hash: crate::firmware::identify(firmware).ok(),
                    firmware: firmware.clone(),
                    eeprom: eeprom.clone(),
                });
                crate::save_preferences(cx);
            }
            if let Some(path) = &presentation.eeprom {
                cx.global_mut::<Desktop>()
                    .preferences
                    .walkers
                    .insert(path.clone(), self.options.clone());
                crate::save_preferences(cx);
            }
        }
        self.presentation = presentation;
        window.set_window_edited(self.presentation.modified);
        if self.closing.pending()
            && self.presentation.closing
            && self.presentation.paused
            && !self.presentation.saving
            && !self.presentation.is_connection_active()
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
        let title = if self.presentation.ready {
            self.filename()
        } else {
            "HachiStep".into()
        };
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
                if !eeprom {
                    self.notify(error, true, cx);
                    if changed {
                        cx.notify();
                    }
                    return;
                }
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
        .with_palette(self.palette_id().filter(|id| {
            cx.global::<Desktop>()
                .preferences
                .palette_named(id)
                .is_some()
        }))
    }

    fn selected_slot(&self, cx: &App) -> u8 {
        self.presentation
            .eeprom
            .as_ref()
            .and_then(|path| cx.global::<Desktop>().preferences.slots.get(path))
            .copied()
            .unwrap_or(1)
    }

    fn select_slot(&mut self, slot: u8, cx: &mut App) {
        if let Some(path) = &self.presentation.eeprom {
            cx.global_mut::<Desktop>()
                .preferences
                .slots
                .insert(path.clone(), slot.clamp(1, 10));
            crate::save_preferences(cx);
            self.install_menu(cx);
        }
    }

    fn slot(&mut self, slot: u8, load: bool, _cx: &mut App) {
        if let Some(eeprom) = &self.presentation.eeprom {
            let path = hs_runtime::slot_path(eeprom, slot);

            self.player.send(if load {
                Command::LoadState(path)
            } else {
                Command::SaveState(path)
            });
        }
    }

    pub(crate) fn install_menu(&mut self, cx: &mut App) {
        let menu = self.menu(cx);
        menu.install(cx);
        if self.menu_state.as_ref() != Some(&menu) {
            self.menu_bar.update(cx, |bar, cx| bar.reload(cx));
            self.menu_state = Some(menu);
        }
    }
    pub fn begin_close(&mut self) {
        if let Some((pending, _, _)) = self.pending.take() {
            pending.send(Command::Close);
        }
        self.closing = Closing::Committed;
        self.player.send(Command::Close);
    }
    pub fn release(&mut self) {
        if let Some(walking) = self.walk_held.take() {
            self.player.send(Command::Walking(walking));
        }
        self.buttons = Buttons::RELEASED;
        if self.presentation.ready {
            self.player.send(Command::Turbo(false));
            self.player.send(Command::Release);
        }
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
        crate::macos::finish_quit(false);
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

    pub fn accepts_file(&self) -> bool {
        !self.presentation.ready && self.pending.is_none() && self.requested.is_none()
    }
    pub(crate) fn focus_window(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
    fn notify(&mut self, text: String, error: bool, cx: &mut Context<Self>) {
        self.notice = Some((
            text,
            Instant::now() + Duration::from_secs(if error { 8 } else { 3 }),
            error,
        ));
        cx.notify();
    }
    fn apply_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preferences = &cx.global::<Desktop>().preferences;
        let frame = self.player.presentation();
        self.options = frame
            .eeprom
            .as_ref()
            .and_then(|path| preferences.walkers.get(path))
            .cloned()
            .unwrap_or_default();
        self.palette_id = self.options.palette.clone();
        self.raw = self.options.raw;
        let speed = self
            .options
            .speed
            .filter(|value| (25..=800).contains(value))
            .unwrap_or(preferences.speed);
        let rate = self
            .options
            .turbo
            .filter(|value| (100..=800).contains(value))
            .unwrap_or(preferences.turbo);
        let cadence = self
            .options
            .cadence
            .filter(|value| (30..=240).contains(value))
            .unwrap_or(preferences.cadence);
        let muted = self.options.muted.unwrap_or(preferences.mute);
        self.player.send(Command::Speed(speed));
        self.player.send(Command::TurboRate(rate));
        self.player.send(Command::Cadence(cadence));
        self.player.send(Command::Mute(muted));
        if self.status != self.options.status {
            self.layout(self.options.status, self.margin, window);
        }
    }
    fn remember_options(&self, cx: &mut App) {
        if let Some(path) = &self.presentation.eeprom {
            cx.global_mut::<Desktop>()
                .preferences
                .walkers
                .insert(path.clone(), self.options.clone());
            crate::save_preferences(cx);
        }
    }
    pub fn palette(&self, cx: &App) -> lcd::Palette {
        let preferences = &cx.global::<Desktop>().preferences;
        let id = self
            .palette_id
            .as_deref()
            .filter(|id| preferences.palette_named(id).is_some())
            .unwrap_or(&preferences.palette_id);
        if let Some((preview, colors)) = &cx.global::<Desktop>().palette_preview {
            if preview == id {
                return *colors;
            }
        }
        self.palette_id
            .as_deref()
            .and_then(|id| preferences.palette_named(id))
            .unwrap_or(preferences.palette)
    }
    pub(crate) fn raw_display(&self) -> bool {
        self.raw
    }
    pub(crate) fn palette_id(&self) -> Option<&str> {
        self.palette_id.as_deref()
    }
    pub fn open_pair(&mut self, pair: Recent, cx: &mut Context<Self>) {
        self.requested = Some(pair);
        cx.notify();
    }
    fn resolve_pair(&mut self, mut pair: Recent, window: &mut Window, cx: &mut Context<Self>) {
        let old_firmware = pair.firmware.clone();
        let old_eeprom = pair.eeprom.clone();
        cx.spawn_in(window, async move |view, cx| {
            for firmware in [false, true] {
                let path = if firmware {
                    &pair.firmware
                } else {
                    &pair.eeprom
                };
                if firmware {
                    let found = crate::firmware::identify(path).ok();
                    if found.is_some()
                        && (pair.firmware_hash.is_none() || found == pair.firmware_hash)
                    {
                        continue;
                    }
                    if let Some(path) = pair
                        .firmware_hash
                        .as_deref()
                        .and_then(crate::firmware::retained)
                    {
                        pair.firmware = path;
                        continue;
                    }
                } else if path.is_file() {
                    continue;
                }
                let filename = settings::filename(path);
                let title = if firmware && path.exists() {
                    "Firmware changed"
                } else if firmware {
                    "Firmware not found"
                } else {
                    "EEPROM not found"
                };
                let action = if firmware {
                    "Locate Firmware…"
                } else {
                    "Locate EEPROM…"
                };
                let Ok(answer) = cx.update(|window, cx| {
                    window.prompt(
                        PromptLevel::Info,
                        title,
                        Some(&filename),
                        &[action, "Cancel"],
                        cx,
                    )
                }) else {
                    return;
                };
                if answer.await.ok() != Some(0) {
                    return;
                }
                let Ok(receiver) = cx.update(|_, cx| {
                    cx.prompt_for_paths(PathPromptOptions {
                        files: true,
                        directories: false,
                        multiple: false,
                        prompt: Some(action.into()),
                    })
                }) else {
                    return;
                };
                let Ok(Ok(Some(paths))) = receiver.await else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                if firmware {
                    let found = crate::firmware::identify(&path);
                    match found {
                        Ok(hash)
                            if pair
                                .firmware_hash
                                .as_ref()
                                .is_none_or(|expected| *expected == hash) =>
                        {
                            pair.firmware = path
                        }
                        _ => {
                            let _ = view.update(cx, |view, cx| {
                                view.error =
                                    Some("Choose the firmware used with this EEPROM.".into());
                                cx.notify();
                            });
                            return;
                        }
                    }
                } else {
                    pair.eeprom = path;
                }
            }
            let _ = cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    for recent in &mut cx.global_mut::<Desktop>().preferences.recent {
                        if recent.firmware == old_firmware
                            && recent.firmware_hash == pair.firmware_hash
                        {
                            recent.firmware = pair.firmware.clone();
                        }
                        if recent.eeprom == old_eeprom {
                            recent.eeprom = pair.eeprom.clone();
                        }
                    }
                    crate::save_preferences(cx);
                    view.open_image(pair.firmware, Some(pair.eeprom), cx);
                })
            });
        })
        .detach();
    }

    pub fn open_image(
        &mut self,
        firmware: PathBuf,
        eeprom: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.release();
        let firmware = if cx
            .global::<Desktop>()
            .preferences
            .default_firmware
            .is_none()
        {
            match crate::firmware::Firmware::retain(&firmware) {
                Ok(retained) => {
                    let path = retained.path().unwrap();
                    cx.global_mut::<Desktop>().preferences.default_firmware = Some(retained);
                    crate::save_preferences(cx);
                    path
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            firmware
        };
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
                    Some(eeprom) => crate::Launch::Files(Recent {
                        firmware_hash: crate::firmware::identify(&firmware).ok(),
                        firmware,
                        eeprom,
                    }),
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
        if crate::system_action(&intent, window, cx) {
            return;
        }
        match intent {
            Intent::NewWindow => crate::new_window(cx, None),
            Intent::Quit => crate::quit(cx),
            Intent::OpenFirmware => self.choose(FileCommand::Firmware, window, cx),
            Intent::OpenEeprom => self.choose(FileCommand::Eeprom, window, cx),
            Intent::Hide
            | Intent::HideOthers
            | Intent::ShowAll
            | Intent::Minimize
            | Intent::Zoom
            | Intent::BringAll => {}
            Intent::Palette(ref id) => {
                self.palette_id = id.clone();
                self.options.palette = id.clone();
                self.remember_options(cx);
            }
            Intent::EditPalettes => crate::preferences::open_display(cx),
            Intent::ClearRecents => {
                let preferences = &mut cx.global_mut::<Desktop>().preferences;
                preferences.recent.clear();
                preferences.recent_firmware.clear();
                crate::save_preferences(cx);
                self.install_menu(cx);
            }
            Intent::RevertEeprom => self.revert(window, cx),
            Intent::NewEeprom => {
                if let Some(firmware) = self.firmware.clone() {
                    self.open_image(firmware, None, cx);
                } else {
                    self.new_after_firmware = true;
                    self.choose(FileCommand::Firmware, window, cx);
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
            Intent::Mute => {
                let muted = !self.presentation.muted;
                self.player.send(Command::Mute(muted));
                self.options.muted = Some(muted);
                self.remember_options(cx);
            }
            Intent::Speed(percent) => {
                self.player.send(Command::Speed(percent));
                self.options.speed = Some(percent);
                self.remember_options(cx);
                self.notify(format!("Speed {percent}%"), false, cx);
            }
            Intent::TurboRate(percent) => {
                self.player.send(Command::TurboRate(percent));
                self.options.turbo = Some(percent);
                self.remember_options(cx);
            }
            Intent::Cadence(cadence) => {
                self.player.send(Command::Cadence(cadence));
                self.options.cadence = Some(cadence);
                self.remember_options(cx);
            }
            Intent::Disconnect => self.player.send(Command::Disconnect),
            Intent::CopyScreen => self.copy_screen(cx),
            Intent::Larger => {
                self.scale = self.scale.saturating_add(1).clamp(1, 8);
                self.fit(window, Some(self.scale));
            }
            Intent::Smaller => {
                self.scale = self.scale.saturating_sub(1).clamp(1, 8);
                self.fit(window, Some(self.scale));
            }
            Intent::Scale(scale) => {
                self.scale = scale;
                self.fit(window, Some(scale));
            }
            Intent::Fit => self.fit(window, None),
            Intent::Fullscreen => window.toggle_fullscreen(),
            Intent::ToggleWalking => self
                .player
                .send(Command::Walking(!self.presentation.walking)),
            Intent::Status => {
                let (status, margin, _) = self.pending_layout.unwrap_or((
                    self.status,
                    self.margin,
                    window.viewport_size(),
                ));
                self.layout(!status, margin, window);
                self.options.status = !status;
                self.remember_options(cx);
            }
            Intent::Raw => {
                self.raw = !self.raw;
                self.options.raw = self.raw;
                self.remember_options(cx);
            }
            Intent::RecentEeprom(pair) => self.open_pair(pair, cx),
            Intent::RecentFirmware(path) => self.selected(FileCommand::Firmware, path, window, cx),
            Intent::Info => (),
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

    fn copy_screen(&mut self, cx: &mut Context<Self>) {
        if !self.presentation.ready {
            return;
        }
        match lcd::screenshot(
            &self.presentation.pixels,
            self.presentation.contrast,
            self.raw,
            self.palette(cx),
        ) {
            Ok(bytes) => {
                cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                    ImageFormat::Png,
                    bytes,
                )));
                self.notify("Copied the screen".into(), false, cx);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((firmware, eeprom)) = self.firmware.clone().zip(self.presentation.eeprom.clone())
        else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Revert \"{}\" to the saved EEPROM?", self.filename()),
            Some("Changes since the last save will be lost and the walker will restart."),
            &["Revert", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |view, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            let _ = view.update(cx, |view, cx| {
                view.release();
                if !view.presentation.paused {
                    view.player.send(Command::Pause);
                }
                let candidate = cx.global::<Desktop>().runtime.player();
                candidate.send(Command::Open {
                    firmware: firmware.clone(),
                    eeprom: Some(eeprom.clone()),
                });
                view.pending = Some((candidate, firmware, Some(eeprom)));
                view.error = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn layout(&mut self, status: bool, margin: u16, window: &mut Window) {
        if window.is_fullscreen() {
            self.status = status;
            self.margin = margin;
            self.pending_layout = None;
            return;
        }
        let (previous_status, previous_margin, mut target) =
            self.pending_layout
                .unwrap_or((self.status, self.margin, window.viewport_size()));
        let delta = px((f32::from(margin) - f32::from(previous_margin)) * 2.);
        target.width += delta;
        target.height += delta + theme::player_chrome(window.rem_size(), status)
            - theme::player_chrome(window.rem_size(), previous_status);
        self.pending_layout = Some((status, margin, target));
        window.resize(target);
    }

    fn fit(&self, window: &mut Window, preset: Option<u8>) {
        if window.is_fullscreen() {
            return;
        }
        let dpi = window.scale_factor();
        let chrome = theme::player_chrome(window.rem_size(), self.status);
        let mut available = window.viewport_size();
        let margin = px(f32::from(self.margin) * 2.);
        available.width -= margin;
        available.height -= chrome + margin;
        let physical = preset.map_or_else(
            || lcd::scale(available, dpi),
            |scale| (f32::from(scale) * dpi).round().max(1.),
        );
        let mut target = lcd::dimensions(physical, dpi);
        target.width += margin;
        target.height += chrome + margin;
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
    pub(crate) fn selected(
        &mut self,
        operation: FileCommand,
        path: PathBuf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match operation {
            FileCommand::Firmware => {
                if !path.is_file() {
                    self.choose(FileCommand::Firmware, _window, cx);
                    return;
                }
                if let Err(error) = crate::firmware::identify(&path) {
                    self.error = Some(error.to_string());
                    return;
                }
                cx.global_mut::<Desktop>()
                    .preferences
                    .remember_firmware(path.clone());
                let path = if cx
                    .global::<Desktop>()
                    .preferences
                    .default_firmware
                    .is_none()
                {
                    match crate::firmware::Firmware::retain(&path) {
                        Ok(retained) => {
                            let path = retained.path().unwrap();
                            cx.global_mut::<Desktop>().preferences.default_firmware =
                                Some(retained);
                            path
                        }
                        Err(error) => {
                            self.error = Some(error.to_string());
                            return;
                        }
                    }
                } else {
                    path
                };
                crate::save_preferences(cx);
                if self.presentation.ready {
                    crate::new_window(cx, Some(crate::Launch::Firmware(path)));
                } else {
                    if let Some((pending, _, _)) = self.pending.take() {
                        pending.send(Command::Close);
                    }
                    self.firmware = Some(path.clone());
                    if std::mem::take(&mut self.new_after_firmware) {
                        self.open_image(path, None, cx);
                    }
                }
            }
            FileCommand::Eeprom => {
                if let Some(firmware) = self.firmware.clone() {
                    self.open_pair(
                        Recent {
                            firmware_hash: crate::firmware::identify(&firmware).ok(),
                            firmware,
                            eeprom: path,
                        },
                        cx,
                    );
                } else {
                    self.requested = Some(Recent {
                        firmware: PathBuf::from("firmware.bin"),
                        eeprom: path,
                        firmware_hash: None,
                    });
                }
            }
            FileCommand::SaveEepromAs => self.player.send(Command::SaveEepromAs(path)),
            FileCommand::SaveState => self.player.send(Command::SaveState(path)),
            FileCommand::LoadState => self.player.send(Command::LoadState(path)),
            FileCommand::Screenshot => match lcd::screenshot(
                &self.presentation.pixels,
                self.presentation.contrast,
                self.raw,
                self.palette(cx),
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
        if key == cx.global::<Desktop>().preferences.walk_key {
            if pressed && self.walk_held.is_none() {
                self.walk_held = Some(self.presentation.walking);
                self.player.send(Command::Walking(true));
            } else if !pressed {
                if let Some(walking) = self.walk_held.take() {
                    self.player.send(Command::Walking(walking));
                }
            }
        }
        self.player.send(Command::Buttons(self.buttons));
    }
    fn empty(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let preferences = &cx.global::<Desktop>().preferences;
        let firmware_name = self.firmware.as_ref().map(|path| {
            preferences
                .default_firmware
                .as_ref()
                .filter(|firmware| {
                    path.file_stem()
                        .is_some_and(|stem| stem == firmware.hash.as_str())
                })
                .map(|firmware| firmware.name.clone())
                .unwrap_or_else(|| settings::filename(path))
        });
        let recent = preferences.recent.clone();
        let compact = window.viewport_size().height < window.rem_size() * 18.;
        v_flex()
            .id("launcher")
            .size_full()
            .items_center()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(32.))
                    .items_center()
                    .gap_3()
                    .p_5()
                    .child(
                        svg()
                            .path("wordmark.svg")
                            .w_48()
                            .h_10()
                            .text_color(cx.theme().primary),
                    )
                    .when(self.pending.is_some(), |root| root.child("Opening…"))
                    .when(self.pending.is_none(), |root| {
                        root.when_some(firmware_name, |root, name| {
                            root.child(
                                h_flex()
                                    .w_full()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .truncate()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(name),
                                    )
                                    .child(
                                        Button::new("change-firmware")
                                            .small()
                                            .label("Change…")
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.execute(Intent::OpenFirmware, window, cx)
                                            })),
                                    ),
                            )
                        })
                        .when(self.firmware.is_none(), |root| {
                            root.child(Button::new("firmware").label("Choose Firmware…").on_click(
                                cx.listener(|view, _, window, cx| {
                                    view.execute(Intent::OpenFirmware, window, cx)
                                }),
                            ))
                        })
                        .when(self.firmware.is_some(), |root| {
                            root.child(
                                h_flex()
                                    .gap_2()
                                    .w_full()
                                    .child(
                                        Button::new("open")
                                            .flex_1()
                                            .label("Open EEPROM…")
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.execute(Intent::OpenEeprom, window, cx)
                                            })),
                                    )
                                    .child(Button::new("new").label("New EEPROM").on_click(
                                        cx.listener(|view, _, window, cx| {
                                            view.execute(Intent::NewEeprom, window, cx)
                                        }),
                                    )),
                            )
                        })
                        .when(!recent.is_empty() && !compact, |root| {
                            root.child(
                                v_flex()
                                    .gap_1()
                                    .w_full()
                                    .mt_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Recent EEPROMs"),
                                    )
                                    .children(recent.into_iter().take(5).enumerate().map(
                                        |(ix, pair)| {
                                            let name = settings::filename(&pair.eeprom);
                                            let folder = pair
                                                .eeprom
                                                .parent()
                                                .map(|path| path.display().to_string())
                                                .unwrap_or_default();
                                            Button::new(("recent", ix))
                                                .accessibility_label(format!("Open {name}"))
                                                .tooltip(pair.eeprom.display().to_string())
                                                .ghost()
                                                .w_full()
                                                .h_auto()
                                                .py_2()
                                                .justify_start()
                                                .child(
                                                    v_flex()
                                                        .w_full()
                                                        .min_w_0()
                                                        .items_start()
                                                        .text_left()
                                                        .child(
                                                            div()
                                                                .w_full()
                                                                .whitespace_normal()
                                                                .child(name),
                                                        )
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .text_sm()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .child(folder),
                                                        ),
                                                )
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.open_pair(pair.clone(), cx)
                                                }))
                                        },
                                    )),
                            )
                        })
                    }),
            )
            .into_any_element()
    }
}

impl Render for PlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.presentation.ready;
        let content = if loaded {
            div()
                .size_full()
                .relative()
                .p(px(f32::from(self.margin)))
                .bg(rgb(self.palette(cx).panel))
                .child(lcd::display(
                    self.presentation.pixels.clone(),
                    self.presentation.contrast,
                    self.raw,
                    self.palette(cx),
                ))
                .when(self.presentation.paused, |root| {
                    let compact = window.viewport_size().width < window.rem_size() * 12.;
                    root.child(
                        div()
                            .absolute()
                            .inset_0()
                            .bg(rgb(self.palette(cx).panel))
                            .opacity(0.55),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                v_flex()
                                    .id("paused-overlay")
                                    .role(Role::Status)
                                    .aria_label("Emulation paused")
                                    .items_center()
                                    .gap_1()
                                    .rounded_md()
                                    .p_2()
                                    .bg(cx.theme().popover)
                                    .text_color(cx.theme().popover_foreground)
                                    .when(!compact, |overlay| {
                                        overlay.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("Paused"),
                                        )
                                    })
                                    .child(
                                        Button::new("resume-emulation")
                                            .xsmall()
                                            .icon(gpui_kit::assets::IconName::Play)
                                            .label("Resume")
                                            .accessibility_label("Resume emulation")
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.execute(Intent::Pause, window, cx);
                                                view.focus.focus(window, cx);
                                            })),
                                    ),
                            ),
                    )
                })
                .into_any_element()
        } else {
            self.empty(window, cx)
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .key_context("EmulatorCommands")
            .on_drop(cx.listener(|_, files: &ExternalPaths, _, cx| {
                let paths = files.0.to_vec();
                cx.defer(move |cx| crate::open_paths(paths, cx));
            }))
            .on_action(cx.listener(|view, action: &Intent, window, cx| {
                view.execute(action.clone(), window, cx)
            }))
            .child(theme::title(
                if loaded {
                    self.filename()
                } else {
                    "HachiStep".into()
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
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus)
                    .key_context("Player")
                    .on_action(
                        cx.listener(|view, _: &component::input::Copy, _, cx| view.copy_screen(cx)),
                    )
                    .on_action(cx.listener(|view, _: &component::input::Undo, _, _| {
                        if view.presentation.can_undo {
                            view.player.send(Command::UndoLoadState);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, window, cx| view.focus.focus(window, cx)),
                    )
                    .when(loaded, |display| {
                        display.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|view, event: &MouseDownEvent, window, cx| {
                                view.release();
                                view.focus.focus(window, cx);
                                let items = view.menu(cx).context();
                                commands::native(items).show(event.position, window, cx);
                            }),
                        )
                    })
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
                    .child(content)
                    .when_some(self.notice.as_ref(), |root, (message, _, error)| {
                        root.child(
                            div()
                                .id("screen-feedback")
                                .role(Role::Status)
                                .aria_label(message.clone())
                                .absolute()
                                .left_2()
                                .max_w(window.viewport_size().width - window.rem_size())
                                .bottom_2()
                                .p_2()
                                .rounded_md()
                                .bg(cx.theme().popover)
                                .text_sm()
                                .text_color(if *error {
                                    cx.theme().danger
                                } else {
                                    cx.theme().popover_foreground
                                })
                                .child(message.clone()),
                        )
                    }),
            )
            .when(self.status, |root| {
                let frame = &self.presentation;
                let compact = window.viewport_size().width < window.rem_size() * 20.;
                let indicators = window.viewport_size().width >= window.rem_size() * 12.;
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
                let percent = if frame.turbo {
                    frame.turbo_percent
                } else {
                    frame.speed_percent
                };
                let pause = if frame.paused { "Resume" } else { "Pause" };
                let numerals = FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]));
                root.child(
                    StatusBar::new()
                        .h(theme::status_height(window.rem_size()))
                        .flex_shrink_0()
                        .px_3()
                        .py_0()
                        .font_features(numerals)
                        .line_height(relative(1.25))
                        .when(loaded, |bar| {
                            bar.left(
                                status_button("status-pause")
                                    .icon(if frame.paused {
                                        gpui_kit::assets::IconName::Play
                                    } else {
                                        gpui_kit::assets::IconName::Pause
                                    })
                                    .accessibility_label(pause)
                                    .tooltip_with_action(pause, &Intent::Pause, None)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.execute(Intent::Pause, window, cx)
                                    })),
                            )
                            .when(!compact, |bar| {
                                bar.left(
                                    component::h_flex()
                                        .gap_1()
                                        .id("emulated-time")
                                        .role(Role::Label)
                                        .aria_label(format!(
                                            "Time since power-on {}",
                                            crate::tools::elapsed(frame.time.raw())
                                        ))
                                        .child(
                                            component::Icon::new(
                                                gpui_kit::assets::IconName::Clock3,
                                            )
                                            .xsmall(),
                                        )
                                        .child(crate::tools::elapsed(frame.time.raw()))
                                        .tooltip(|window, cx| {
                                            component::tooltip::Tooltip::new("Time since power-on")
                                                .build(window, cx)
                                        }),
                                )
                            })
                            .when(indicators && frame.modified, |bar| {
                                bar.right(
                                    status_button("status-save")
                                        .icon(gpui_kit::assets::IconName::Save)
                                        .accessibility_label("Save EEPROM")
                                        .tooltip_with_action(
                                            "Save EEPROM",
                                            &Intent::SaveEeprom,
                                            None,
                                        )
                                        .disabled(frame.saving)
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.execute(Intent::SaveEeprom, window, cx)
                                        })),
                                )
                            })
                            .when(indicators && frame.walking, |bar| {
                                bar.right(
                                    status_button("status-walking")
                                        .icon(gpui_kit::assets::IconName::Footprints)
                                        .accessibility_label("Walking")
                                        .tooltip(format!("Walking · {}/min", frame.cadence))
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.execute(
                                                Intent::Tool(crate::tools::Kind::Motion),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                            })
                            .when(indicators && !connection.is_empty(), |bar| {
                                bar.right(
                                    status_button("status-connection")
                                        .icon(
                                            if matches!(
                                                frame.connection,
                                                hs_runtime::Connection::Error(_)
                                            ) {
                                                gpui_kit::assets::IconName::TriangleAlert
                                            } else if frame.peer.is_some() {
                                                gpui_kit::assets::IconName::Radio
                                            } else {
                                                gpui_kit::assets::IconName::Network
                                            },
                                        )
                                        .accessibility_label(connection.clone())
                                        .when(
                                            !compact
                                                && frame.connection
                                                    != hs_runtime::Connection::Connected
                                                && frame.peer.is_none(),
                                            |button| {
                                                button.label(match frame.connection {
                                                    hs_runtime::Connection::Error(_) => {
                                                        "IR error".into()
                                                    }
                                                    _ => frame.connection_name(),
                                                })
                                            },
                                        )
                                        .tooltip(connection)
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            let kind = crate::tools::Kind::Infrared;
                                            view.execute(Intent::Tool(kind), window, cx)
                                        })),
                                )
                            })
                            .right(
                                status_button("status-speed")
                                    .when(!compact, |button| {
                                        button
                                            .icon(gpui_kit::assets::IconName::Gauge)
                                            .dropdown_caret(true)
                                    })
                                    .label(format!("{percent}%"))
                                    .w(if compact { rems(3.5) } else { rems(5.) })
                                    .accessibility_label("Emulation speed")
                                    .tooltip(format!(
                                        "Speed {}%. Measured {:.0}%.",
                                        frame.speed_percent,
                                        frame.speed * 100.
                                    ))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.release();
                                        view.focus.focus(window, cx);
                                        let compact =
                                            window.viewport_size().width < window.rem_size() * 20.;
                                        let at = point(
                                            window.viewport_size().width
                                                - window.rem_size()
                                                    * (if compact { 3.5 } else { 5. } + 0.75),
                                            window.viewport_size().height,
                                        );
                                        commands::native(view.menu(cx).speeds())
                                            .show(at, window, cx);
                                    })),
                            )
                        }),
                )
            })
    }
}
