use crate::{
    commands::{self, Intent},
    settings::{self, Preferences},
    Desktop,
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        h_flex,
        kbd::Kbd,
        menu::{DropdownMenu, PopupMenuItem},
        switch::Switch,
        v_flex, ActiveTheme, Selectable, Sizable,
    },
    prelude::*,
    *,
};

pub(crate) fn open(cx: &mut App) {
    cx.defer(|cx| open_section(None, cx));
}
pub(crate) fn open_controls(cx: &mut App) {
    cx.defer(|cx| open_section(Some(Section::Controls), cx));
}
pub(crate) fn open_display(cx: &mut App) {
    cx.defer(|cx| open_section(Some(Section::Display), cx));
}
fn open_section(section: Option<Section>, cx: &mut App) {
    if let Some((handle, view)) = cx.global::<Desktop>().preferences_window.clone() {
        if handle
            .update(cx, |_, window, cx| {
                let _ = view.update(cx, |view, cx| {
                    if let Some(section) = section {
                        view.section = section;
                        view._resize =
                            crate::macos::resize_settings(window, px(section.height()), cx);
                    }
                    view.capture = None;
                    cx.notify();
                });
                window.activate_window();
            })
            .is_ok()
        {
            return;
        }
    }
    let section = section.unwrap_or(
        [
            Section::Display,
            Section::Controls,
            Section::General,
            Section::Hotkeys,
        ][cx.global::<Desktop>().preferences.settings_page.min(3)],
    );
    let mut options = crate::options("Settings", 520., section.height());
    options.window_min_size = Some(size(px(500.), px(280.)));
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Settings::new(section, window, cx))
    }) {
        Ok((handle, view)) => {
            cx.global_mut::<Desktop>().preferences_window = Some((handle, view.downgrade()))
        }
        Err(error) => eprintln!("Could not open Settings: {error:#}"),
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Section {
    Display,
    Controls,
    General,
    Hotkeys,
}
impl Section {
    fn height(self) -> f32 {
        match self {
            Self::General => 490.,
            Self::Display => 470.,
            Self::Controls => 390.,
            Self::Hotkeys => 560.,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Display => "Display",
            Self::Controls => "Controls",
            Self::General => "General",
            Self::Hotkeys => "Hotkeys",
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Capture {
    Button(usize),
    FastHold,
    WalkHold,
    Shortcut(usize),
    Gamepad(usize),
}
impl Capture {
    fn name(self) -> String {
        match self {
            Self::Button(ix) => ["Left", "Center", "Right"][ix].into(),
            Self::FastHold => "Fast-forward while held".into(),
            Self::WalkHold => "Walk while held".into(),
            Self::Shortcut(ix) => commands::shortcuts()[ix].label.into(),
            Self::Gamepad(ix) => format!("{} gamepad button", ["Left", "Center", "Right"][ix]),
        }
    }
    fn key(self, p: &Preferences) -> String {
        match self {
            Self::Button(ix) => p.keys[ix].clone(),
            Self::FastHold => p.turbo_key.clone(),
            Self::WalkHold => p.walk_key.clone(),
            Self::Shortcut(ix) => commands::shortcuts()[ix].key(p),
            Self::Gamepad(ix) => settings::GAMEPAD[p.gamepad[ix].min(settings::GAMEPAD.len() - 1)]
                .1
                .into(),
        }
    }
    fn set(self, p: &mut Preferences, key: String) {
        match self {
            Self::Button(ix) => p.keys[ix] = key,
            Self::FastHold => p.turbo_key = key,
            Self::WalkHold => p.walk_key = key,
            Self::Shortcut(ix) => {
                p.hotkeys.insert(commands::shortcuts()[ix].id.into(), key);
            }
            Self::Gamepad(_) => (),
        }
    }
    fn accepts(self, key: &str) -> bool {
        match self {
            Self::Shortcut(_) => true,
            Self::FastHold | Self::WalkHold if key.is_empty() => true,
            Self::Button(_) | Self::FastHold | Self::WalkHold => {
                Keystroke::parse(key).is_ok_and(|key| !key.modifiers.modified())
            }
            Self::Gamepad(_) => false,
        }
    }
}
struct Conflict {
    target: Capture,
    key: String,
    other: Capture,
}
pub(crate) struct Settings {
    section: Section,
    focus: FocusHandle,
    capture: Option<Capture>,
    conflict: Option<Conflict>,
    error: Option<String>,
    palettes: Entity<crate::palettes::Editor>,
    firmware_error: Option<String>,
    target: Option<WeakEntity<crate::player::PlayerView>>,
    pad_sequence: u64,
    _subscriptions: Vec<Subscription>,
    _resize: Option<Task<()>>,
}
impl Settings {
    fn new(section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let release = cx.on_release(|_, cx| {
            cx.global_mut::<Desktop>().preferences_window = None;
            cx.global_mut::<Desktop>().palette_preview = None;
            cx.refresh_windows();
        });
        let palettes = cx.new(|cx| crate::palettes::Editor::new(window, cx));
        let mut subscriptions = vec![release];
        let target = cx.global::<Desktop>().active_player.clone();
        subscriptions.push(cx.observe_window_activation(window, |view, window, cx| {
            view.capture = None;
            if window.is_window_active() {
                view.target = cx.global::<Desktop>().active_player.clone();
                if let Some(target) = &view.target {
                    crate::player_menu(target, cx);
                }
            }
        }));
        let owner = cx.weak_entity();
        let id = window.window_handle().window_id();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle().window_id() != id {
                return;
            }
            let _ = owner.update(cx, |view, cx| {
                if view
                    .capture
                    .is_some_and(|capture| !matches!(capture, Capture::Gamepad(_)))
                {
                    cx.stop_propagation();
                    view.capture_key(&event.keystroke, cx);
                }
            });
        }));
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
            if cx.update(|window, cx| view.update(cx, |view, cx| {
                if window.is_window_active() {
                    if let Some(Capture::Gamepad(ix)) = view.capture {
                        let press = cx.global::<Desktop>().windows.iter().filter_map(|w| w.view.upgrade()).find_map(|view| view.read(cx).player().presentation().gamepad_press);
                        if let Some((sequence, button))=press.filter(|(sequence,_)| *sequence != view.pad_sequence) {
                            view.pad_sequence=sequence;
                            if let Some(choice)=settings::GAMEPAD.iter().position(|(value,_)| *value==button) {
                                view.update(|p| {
                                    let old=p.gamepad[ix];
                                    if let Some(other)=p.gamepad.iter().enumerate().find_map(|(other,value)| (other!=ix && *value==choice).then_some(other)) {p.gamepad[other]=old;}
                                    p.gamepad[ix]=choice;
                                }, cx);
                                view.apply_gamepad(cx); view.capture=None;
                            } else {view.error=Some("Choose a face button, shoulder button, or D-pad direction.".into());}
                            cx.notify();
                        }
                    }
                    if let Some(target)=&view.target {crate::player_menu(target, cx);}
                }
            })).is_err() {break;}
        }).detach();
        Self {
            section,
            focus,
            capture: None,
            conflict: None,
            error: None,
            palettes,
            firmware_error: None,
            target,
            pad_sequence: 0,
            _subscriptions: subscriptions,
            _resize: None,
        }
    }
    fn update(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        change(&mut cx.global_mut::<Desktop>().preferences);
        crate::save_preferences(cx);
        cx.refresh_windows();
        cx.notify();
    }
    fn apply_gamepad(&self, cx: &App) {
        let mapping = cx
            .global::<Desktop>()
            .preferences
            .gamepad
            .map(|ix| settings::GAMEPAD[ix.min(settings::GAMEPAD.len() - 1)].0);
        cx.global::<Desktop>().runtime.gamepad_mapping(mapping);
    }
    fn changed_keys(&mut self, cx: &mut Context<Self>) {
        commands::bind(cx);
        let players: Vec<_> = cx
            .global::<Desktop>()
            .windows
            .iter()
            .filter_map(|w| w.view.upgrade())
            .collect();
        for player in players {
            player.update(cx, |view, cx| {
                view.release();
                view.install_menu(cx);
            });
        }
        crate::save_preferences(cx);
        cx.notify();
    }
    fn captures() -> Vec<Capture> {
        [
            Capture::Button(0),
            Capture::Button(1),
            Capture::Button(2),
            Capture::FastHold,
            Capture::WalkHold,
        ]
        .into_iter()
        .chain((0..commands::shortcuts().len()).map(Capture::Shortcut))
        .collect()
    }
    fn capture_key(&mut self, key: &Keystroke, cx: &mut Context<Self>) {
        let Some(target) = self.capture else {
            return;
        };
        if key.key == "escape" {
            self.capture = None;
            cx.notify();
            return;
        }
        if matches!(
            key.key.as_str(),
            "shift" | "control" | "alt" | "command" | "super" | "fn"
        ) {
            return;
        }
        let key = if key.key == "backspace" {
            String::new()
        } else {
            key.unparse()
        };
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        let reserved = [
            "n", "o", "shift-o", "s", "shift-s", "w", "q", ",", "=", "-", "/", "alt-i",
        ];
        let fixed = reserved.iter().any(|reserved| {
            Keystroke::parse(&format!("{modifier}-{reserved}")).ok() == Keystroke::parse(&key).ok()
        }) || (1..=10)
            .any(|slot| key == format!("f{slot}") || key == format!("shift-f{slot}"));
        if fixed || !target.accepts(&key) {
            self.error = Some(
                if fixed {
                    "That shortcut is reserved for a menu command."
                } else {
                    "Use one key without modifiers for a held control or walker button."
                }
                .into(),
            );
            cx.notify();
            return;
        }
        self.capture = None;
        self.error = None;
        let p = &cx.global::<Desktop>().preferences;
        if let Some(other) = Self::captures().into_iter().find(|other| {
            *other != target
                && !key.is_empty()
                && Keystroke::parse(&other.key(p)).ok() == Keystroke::parse(&key).ok()
        }) {
            self.conflict = Some(Conflict { target, key, other });
            cx.notify();
            return;
        }
        self.update(|p| target.set(p, key), cx);
        self.changed_keys(cx);
    }
    fn key_button(&self, target: Capture, cx: &Context<Self>) -> Button {
        let binding = target.key(&cx.global::<Desktop>().preferences);
        let capturing = self.capture == Some(target);
        Button::new(SharedString::from(format!("binding-{}", target.name())))
            .small()
            .w_40()
            .accessibility_label(format!("Change {} binding", target.name()))
            .when(capturing, |button| {
                button.label(if matches!(target, Capture::Gamepad(_)) {
                    "Press a button…"
                } else {
                    "Press a key…"
                })
            })
            .when(!capturing, |button| {
                if matches!(target, Capture::Gamepad(_)) || binding.is_empty() {
                    button.label(if binding.is_empty() {
                        "None".into()
                    } else {
                        binding.clone()
                    })
                } else {
                    button.child(
                        Kbd::new(Keystroke::parse(&binding).unwrap_or_default()).appearance(false),
                    )
                }
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.capture = Some(target);
                view.conflict = None;
                view.error = None;
                view.pad_sequence = cx
                    .global::<Desktop>()
                    .windows
                    .iter()
                    .filter_map(|w| w.view.upgrade())
                    .find_map(|view| {
                        view.read(cx)
                            .player()
                            .presentation()
                            .gamepad_press
                            .map(|(sequence, _)| sequence)
                    })
                    .unwrap_or(0);
                view.focus.focus(window, cx);
                cx.notify();
            }))
    }
    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_4()
                    .child(div().w_16())
                    .child(div().w_40().text_sm().child("Keyboard"))
                    .child(div().w_40().text_sm().child("Gamepad")),
            )
            .children(
                ["Left", "Center", "Right"]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, name)| {
                        h_flex()
                            .gap_4()
                            .child(div().w_16().child(name))
                            .child(self.key_button(Capture::Button(ix), cx))
                            .child(self.key_button(Capture::Gamepad(ix), cx))
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a binding, then press the key or gamepad button."),
            )
            .into_any_element()
    }
    fn hotkeys(&self, cx: &mut Context<Self>) -> AnyElement {
        let shortcuts = commands::shortcuts();
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .font_weight(FontWeight::MEDIUM)
                            .child("Held controls"),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_3()
                            .child("Fast-forward while held")
                            .child(self.key_button(Capture::FastHold, cx)),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_3()
                            .child("Walk while held")
                            .child(self.key_button(Capture::WalkHold, cx)),
                    ),
            )
            .children(["System", "States", "Screen"].map(|group| {
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .font_weight(FontWeight::MEDIUM)
                            .child(group),
                    )
                    .children(
                        shortcuts
                            .iter()
                            .enumerate()
                            .filter(|(_, shortcut)| shortcut.group == group)
                            .map(|(ix, shortcut)| {
                                h_flex()
                                    .gap_3()
                                    .justify_between()
                                    .child(shortcut.label)
                                    .child(self.key_button(Capture::Shortcut(ix), cx))
                            }),
                    )
            }))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "F1–F10 load slots. Shift-F1–F10 save slots. Backspace clears a shortcut.",
                    ),
            )
            .child(
                Button::new("restore-hotkeys")
                    .small()
                    .self_end()
                    .label("Restore All Bindings")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.update(
                            |p| {
                                p.hotkeys.clear();
                                p.turbo_key = "space".into();
                                p.walk_key = "w".into();
                                let defaults = Preferences::default();
                                p.keys = defaults.keys;
                                p.gamepad = defaults.gamepad;
                            },
                            cx,
                        );
                        view.changed_keys(cx);
                        view.apply_gamepad(cx);
                    })),
            )
            .into_any_element()
    }
    fn choose_firmware(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |view, cx| {
            let Ok(receiver) = cx.update(|_, cx| {
                cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Choose Firmware".into()),
                })
            }) else {
                return;
            };
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            if let Some(path) = paths.first() {
                let result = crate::firmware::Firmware::retain(path);
                let _ = view.update(cx, |view, cx| match result {
                    Ok(firmware) => {
                        view.firmware_error = None;
                        view.update(|p| p.default_firmware = Some(firmware), cx);
                    }
                    Err(error) => {
                        view.firmware_error = Some(error.to_string());
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
    fn general(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = &cx.global::<Desktop>().preferences;
        let owner = cx.weak_entity();
        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_3()
                    .child(div().font_weight(FontWeight::MEDIUM).child("Firmware"))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                div().flex_1().min_w_0().child(
                                    p.default_firmware
                                        .as_ref()
                                        .map(|firmware| firmware.name.clone())
                                        .unwrap_or_else(|| "None selected".into()),
                                ),
                            )
                            .child(
                                Button::new("default-firmware")
                                    .small()
                                    .label(if p.default_firmware.is_some() {
                                        "Change…"
                                    } else {
                                        "Choose…"
                                    })
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.choose_firmware(window, cx)
                                    })),
                            )
                            .when(p.default_firmware.is_some(), |row| {
                                row.child(
                                    Button::new("clear-firmware")
                                        .small()
                                        .label("Clear")
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.update(|p| p.default_firmware = None, cx)
                                        })),
                                )
                            }),
                    )
                    .child(div().text_sm().text_color(cx.theme().muted_foreground)
                        .child("New walkers use this firmware. Existing EEPROMs remember the firmware they last used."))
                    .when_some(self.firmware_error.clone(), |root, error| {
                        root.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
            .child(component::separator::Separator::horizontal())
            .child(
                v_flex()
                    .gap_3()
                    .child(div().font_weight(FontWeight::MEDIUM).child("New windows"))
                    .child(self.speed_choice("default-speed", "Speed", p.speed, false, cx))
                    .child(self.speed_choice("turbo-speed", "Fast-forward rate", p.turbo, true, cx))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child("Walking pace")
                            .child(
                                Button::new("default-pace")
                                    .small()
                                    .w_40()
                                    .label(format!("{} steps/min", p.cadence))
                                    .dropdown_menu(move |mut menu, _, _| {
                                        for pace in [80, 100, 120, 140, 160] {
                                            let owner = owner.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(format!("{pace} steps/min"))
                                                    .on_click(move |_, _, cx| {
                                                        let _ = owner.update(cx, |view, cx| {
                                                            view.update(|p| p.cadence = pace, cx)
                                                        });
                                                    }),
                                            );
                                        }
                                        menu
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child("Start muted")
                            .child(
                                Switch::new("start-muted")
                                    .accessibility_label("Start new windows muted")
                                    .checked(p.mute)
                                    .on_change(cx.listener(|view, muted: &bool, _, cx| {
                                        view.update(|p| p.mute = *muted, cx)
                                    })),
                            ),
                    ),
            )
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("These defaults apply to new walkers. Change an open walker's settings in the System menu."))
            .when_some(
                cx.global::<Desktop>()
                    .windows
                    .iter()
                    .filter_map(|window| window.view.upgrade())
                    .find_map(|view| view.read(cx).presentation().audio_error.clone()),
                |root, error| {
                    root.child(
                        v_flex()
                            .gap_2()
                            .child(div().text_sm().text_color(cx.theme().danger).child(error))
                            .child(
                                Button::new("retry-audio")
                                    .small()
                                    .label("Retry Audio")
                                    .on_click(|_, _, cx| {
                                        cx.global::<Desktop>().runtime.retry_audio()
                                    }),
                            ),
                    )
                },
            )
            .into_any_element()
    }
    fn speed_choice(
        &self,
        id: &'static str,
        label: &'static str,
        value: u16,
        turbo: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        h_flex()
            .justify_between()
            .gap_4()
            .child(label)
            .child(
                Button::new(id)
                    .small()
                    .w_40()
                    .label(format!("{value}%"))
                    .dropdown_menu(move |mut menu, _, _| {
                        for percent in [50, 75, 100, 150, 200, 400, 800] {
                            if turbo && percent < 100 {
                                continue;
                            }
                            let owner = owner.clone();
                            menu = menu.item(
                                PopupMenuItem::new(format!("{percent}%"))
                                    .checked(percent == value)
                                    .on_click(move |_, _, cx| {
                                        let _ = owner.update(cx, |view, cx| {
                                            view.update(
                                                |p| {
                                                    if turbo {
                                                        p.turbo = percent
                                                    } else {
                                                        p.speed = percent
                                                    }
                                                },
                                                cx,
                                            )
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .into_any_element()
    }
}
impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.section {
            Section::Display => self.palettes.clone().into_any_element(),
            Section::Controls => self.controls(cx),
            Section::General => self.general(cx),
            Section::Hotkeys => self.hotkeys(cx),
        };
        let sections = [
            Section::General,
            Section::Display,
            Section::Controls,
            Section::Hotkeys,
        ];
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .track_focus(&self.focus)
            .key_context("EmulatorCommands")
            .on_action(cx.listener(|view, action: &Intent, window, cx| {
                if crate::system_action(action, window, cx) {
                    return;
                }
                match action {
                    Intent::Close => window.remove_window(),
                    Intent::Quit => crate::quit(cx),
                    Intent::Settings => (),
                    _ => {
                        if let Some(target) = &view.target {
                            crate::invoke_player(target, action.clone(), cx);
                        } else {
                            crate::application_action(action, cx);
                        }
                    }
                }
            }))
            .capture_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && view.capture.is_some() {
                    view.capture = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                gpui_kit::component::TitleBar::new()
                    .relative()
                    .when(cfg!(target_os = "macos"), |bar| bar.pl_0())
                    .bg(cx.theme().background)
                    .border_b_0()
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if window.is_window_active() {
                                cx.theme().foreground
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child("Settings"),
                    ),
            )
            .child(
                h_flex()
                    .justify_center()
                    .gap_1()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(
                        sections
                            .into_iter()
                            .zip([
                                gpui_kit::assets::IconName::Settings,
                                gpui_kit::assets::IconName::Monitor,
                                gpui_kit::assets::IconName::Gamepad2,
                                gpui_kit::assets::IconName::Keyboard,
                            ])
                            .map(|(section, icon)| {
                                let selected = section == self.section;
                                Button::new(SharedString::from(format!(
                                    "settings-{}",
                                    section.name()
                                )))
                                .ghost()
                                .w_20()
                                .h_12()
                                .selected(selected)
                                .border_1()
                                .border_color(gpui_kit::transparent_black())
                                .when(selected, |button| {
                                    button.bg(cx.theme().muted).border_color(cx.theme().border)
                                })
                                .accessibility_label(section.name())
                                .child(
                                    v_flex()
                                        .items_center()
                                        .gap_1()
                                        .text_xs()
                                        .text_color(if selected {
                                            cx.theme().primary
                                        } else {
                                            cx.theme().muted_foreground
                                        })
                                        .child(component::Icon::new(icon).size_5())
                                        .child(section.name()),
                                )
                                .on_click(cx.listener(
                                    move |view, _, window, cx| {
                                        view.section = section;
                                        view._resize = crate::macos::resize_settings(
                                            window,
                                            px(section.height()),
                                            cx,
                                        );
                                        view.capture = None;
                                        view.conflict = None;
                                        view.error = None;
                                        view.update(
                                            |p| p.settings_page = view_section_index(section),
                                            cx,
                                        );
                                    },
                                ))
                            }),
                    ),
            )
            .child(
                div()
                    .id("settings-pane")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .px_5()
                            .py_3()
                            .gap_3()
                            .child(content)
                            .when(self.capture.is_some(), |root| {
                                root.child(
                                    Button::new("cancel-capture")
                                        .small()
                                        .self_end()
                                        .label("Cancel")
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.capture = None;
                                            cx.notify();
                                        })),
                                )
                            })
                            .when_some(self.conflict.as_ref(), |root, conflict| {
                                let p = &cx.global::<Desktop>().preferences;
                                let old = conflict.target.key(p);
                                let swappable = conflict.other.accepts(&old);
                                root.child(
                                    v_flex()
                                        .gap_2()
                                        .child(format!(
                                            "{} is already used by {}.",
                                            settings::key_name(&conflict.key),
                                            conflict.other.name()
                                        ))
                                        .child(
                                            h_flex()
                                                .justify_end()
                                                .gap_2()
                                                .child(
                                                    Button::new("cancel-swap")
                                                        .small()
                                                        .label("Cancel")
                                                        .on_click(cx.listener(|view, _, _, cx| {
                                                            view.conflict = None;
                                                            cx.notify();
                                                        })),
                                                )
                                                .when(swappable, |row| {
                                                    row.child(
                                                        Button::new("swap-bindings")
                                                            .small()
                                                            .label("Swap")
                                                            .on_click(cx.listener(
                                                                |view, _, _, cx| {
                                                                    if let Some(conflict) =
                                                                        view.conflict.take()
                                                                    {
                                                                        let old = conflict
                                                                            .target
                                                                            .key(
                                                                            &cx.global::<Desktop>()
                                                                                .preferences,
                                                                        );
                                                                        view.update(
                                                                            |p| {
                                                                                conflict
                                                                                    .target
                                                                                    .set(
                                                                                        p,
                                                                                        conflict
                                                                                            .key,
                                                                                    );
                                                                                conflict
                                                                                    .other
                                                                                    .set(p, old);
                                                                            },
                                                                            cx,
                                                                        );
                                                                        view.changed_keys(cx);
                                                                    }
                                                                },
                                                            )),
                                                    )
                                                }),
                                        ),
                                )
                            })
                            .when_some(self.error.clone(), |root, error| {
                                root.child(
                                    div().text_sm().text_color(cx.theme().danger).child(error),
                                )
                            }),
                    ),
            )
    }
}
fn view_section_index(section: Section) -> usize {
    match section {
        Section::Display => 0,
        Section::Controls => 1,
        Section::General => 2,
        Section::Hotkeys => 3,
    }
}
