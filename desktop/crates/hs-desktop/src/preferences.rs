use crate::{
    commands::Intent,
    settings::{self, Preferences},
    theme, Desktop,
};
use gpui_kit::{
    component::{
        button::Button,
        h_flex,
        kbd::Kbd,
        menu::{DropdownMenu, PopupMenuItem},
        switch::Switch,
        tab::{Tab, TabBar},
        v_flex, ActiveTheme, Sizable,
    },
    prelude::*,
    *,
};

const WIDTH: f32 = 460.;
const HEIGHT: f32 = 420.;

pub(crate) fn open(cx: &mut App) {
    cx.defer(|cx| open_section(None, cx));
}
pub(crate) fn open_controls(cx: &mut App) {
    cx.defer(|cx| open_section(Some(Section::Controls), cx));
}
fn open_section(section: Option<Section>, cx: &mut App) {
    if let Some((handle, view)) = cx.global::<Desktop>().preferences_window.clone() {
        if handle
            .update(cx, |_, window, cx| {
                let _ = view.update(cx, |view, cx| {
                    if let Some(section) = section {
                        view.section = section;
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
        [Section::Appearance, Section::Controls, Section::Emulation]
            [cx.global::<Desktop>().preferences.settings_page.min(2)],
    );
    let mut options = crate::options("Settings", WIDTH, HEIGHT);
    options.is_resizable = false;
    options.window_min_size = None;
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
    Appearance,
    Controls,
    Emulation,
}
impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Controls => "Controls",
            Self::Emulation => "Emulation",
        }
    }
}

pub(crate) struct Settings {
    section: Section,
    focus: FocusHandle,
    capture: Option<usize>,
    error: Option<&'static str>,
    palettes: Entity<crate::palettes::Editor>,
    firmware_error: Option<String>,
    target: Option<WeakEntity<crate::player::PlayerView>>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    fn new(section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let release = cx.on_release(|_, cx| cx.global_mut::<Desktop>().preferences_window = None);
        let palettes = cx.new(|cx| crate::palettes::Editor::new(window, cx));
        let mut subscriptions = vec![release];
        let target = cx.global::<Desktop>().active_player.clone().or_else(|| {
            cx.global::<Desktop>()
                .windows
                .last()
                .map(|w| w.view.clone())
        });
        subscriptions.push(cx.observe_window_activation(window, |view, window, cx| {
            if window.is_window_active() {
                view.target = cx.global::<Desktop>().active_player.clone();
                if let Some(target) = &view.target {
                    crate::player_menu(target, cx);
                }
            }
        }));
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            if cx
                .update(|window, cx| {
                    view.update(cx, |view, cx| {
                        if window.is_window_active() {
                            if let Some(target) = &view.target {
                                crate::player_menu(target, cx);
                            }
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
            section,
            focus,
            capture: None,
            error: None,
            palettes,
            firmware_error: None,
            target,
            _subscriptions: subscriptions,
        }
    }
    fn update(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        let previous = cx.global::<Desktop>().preferences.turbo;
        change(&mut cx.global_mut::<Desktop>().preferences);
        let rate = cx.global::<Desktop>().preferences.turbo;
        if previous != rate {
            for view in cx
                .global::<Desktop>()
                .windows
                .iter()
                .filter_map(|window| window.view.upgrade())
            {
                view.read(cx)
                    .player()
                    .send(hs_runtime::Command::TurboRate(rate));
            }
        }
        crate::save_preferences(cx);
        cx.refresh_windows();
        cx.notify();
    }
    fn appearance(&self, _: &mut Context<Self>) -> AnyElement {
        self.palettes.clone().into_any_element()
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
    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let preferences = cx.global::<Desktop>().preferences.clone();
        let mut keyboard = v_flex().gap_2().child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Keyboard"),
        );
        let mut gamepad = v_flex().gap_2().child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Gamepad"),
        );
        for (ix, label) in ["Left", "Center", "Right"].into_iter().enumerate() {
            keyboard = keyboard.child(self.key_row(label, ix, &preferences.keys[ix], cx));
            let owner = cx.weak_entity();
            gamepad = gamepad.child(
                h_flex().justify_between().gap_4().child(label).child(
                    Button::new(("gamepad", ix))
                        .small()
                        .w_40()
                        .label(
                            settings::GAMEPAD
                                [preferences.gamepad[ix].min(settings::GAMEPAD.len() - 1)]
                            .1,
                        )
                        .dropdown_menu(move |mut menu, _, _| {
                            for (choice, (_, name)) in settings::GAMEPAD.iter().enumerate() {
                                let owner = owner.clone();
                                menu = menu.item(PopupMenuItem::new(*name).on_click(
                                    move |_, _, cx| {
                                        let _ = owner.update(cx, |view, cx| {
                                            view.update(|p| p.gamepad[ix] = choice, cx);
                                            let mapping =
                                                cx.global::<Desktop>().preferences.gamepad.map(
                                                    |ix| {
                                                        settings::GAMEPAD
                                                            [ix.min(settings::GAMEPAD.len() - 1)]
                                                        .0
                                                    },
                                                );
                                            cx.global::<Desktop>().runtime.gamepad_mapping(mapping);
                                        });
                                    },
                                ));
                            }
                            menu
                        }),
                ),
            );
        }
        keyboard = keyboard.child(self.key_row("Fast-forward", 3, &preferences.turbo_key, cx));
        v_flex()
            .gap_6()
            .child(keyboard)
            .child(gamepad)
            .into_any_element()
    }
    fn key_row(&self, label: &str, index: usize, binding: &str, cx: &Context<Self>) -> AnyElement {
        let capture = self.capture == Some(index);
        h_flex()
            .justify_between()
            .gap_4()
            .child(label.to_owned())
            .child(
                Button::new(("key", index))
                    .small()
                    .w_40()
                    .accessibility_label(format!("Change {label} binding"))
                    .when(capture, |button| button.label("Press a key"))
                    .when(!capture, |button| {
                        button.child(
                            Kbd::new(Keystroke::parse(binding).unwrap_or_default())
                                .appearance(false),
                        )
                    })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.capture = Some(index);
                        view.error = None;
                        view.focus.focus(window, cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn emulation(&self, cx: &mut Context<Self>) -> AnyElement {
        let preferences = &cx.global::<Desktop>().preferences;
        let default = preferences
            .default_firmware
            .as_ref()
            .map(|f| f.name.clone());
        let owner = cx.weak_entity();
        let firmware = if let Some(name) = default {
            Button::new("default-firmware")
                .w_40()
                .small()
                .label(name)
                .accessibility_label("Default firmware")
                .child(component::Icon::new(component::IconName::ChevronDown).xsmall())
                .dropdown_menu(move |menu, _, _| {
                    let choose = owner.clone();
                    let clear = owner.clone();
                    menu.item(
                        PopupMenuItem::new("Change…").on_click(move |_, window, cx| {
                            let _ = choose.update(cx, |view, cx| view.choose_firmware(window, cx));
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Clear Default").on_click(move |_, _, cx| {
                            let _ = clear.update(cx, |view, cx| {
                                view.update(|p| p.default_firmware = None, cx)
                            });
                        }),
                    )
                })
                .into_any_element()
        } else {
            Button::new("default-firmware")
                .w_40()
                .small()
                .label("Choose…")
                .on_click(cx.listener(|view, _, window, cx| view.choose_firmware(window, cx)))
                .into_any_element()
        };
        let heading = |text: &'static str| {
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(text)
        };
        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_3()
                    .child(heading("Firmware"))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child("Default")
                            .child(firmware),
                    )
                    .when_some(self.firmware_error.clone(), |root, error| {
                        root.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
            .child(component::separator::Separator::horizontal())
            .child(
                v_flex()
                    .gap_3()
                    .child(heading("Playback"))
                    .child(self.speed_choice(
                        "default-speed",
                        "Default speed",
                        preferences.speed,
                        false,
                        cx,
                    ))
                    .child(self.speed_choice(
                        "turbo-speed",
                        "Fast-forward speed",
                        preferences.turbo,
                        true,
                        cx,
                    )),
            )
            .child(component::separator::Separator::horizontal())
            .child(
                v_flex()
                    .gap_3()
                    .child(heading("Audio"))
                    .child(
                        h_flex().justify_between().gap_4().child("Sound").child(
                            Switch::new("sound")
                                .accessibility_label("Sound")
                                .checked(!preferences.mute)
                                .on_change(cx.listener(|view, enabled: &bool, _, cx| {
                                    view.update(|p| p.mute = !*enabled, cx);
                                    cx.global::<Desktop>().runtime.sound_enabled(*enabled);
                                })),
                        ),
                    )
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
                                    .child(
                                        div().text_sm().text_color(cx.theme().danger).child(error),
                                    )
                                    .child(
                                        Button::new("retry-audio").small().label("Retry").on_click(
                                            |_, _, cx| cx.global::<Desktop>().runtime.retry_audio(),
                                        ),
                                    ),
                            )
                        },
                    ),
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
            Section::Appearance => self.appearance(cx),
            Section::Controls => self.controls(cx),
            Section::Emulation => self.emulation(cx),
        };
        let sections = [Section::Appearance, Section::Controls, Section::Emulation];
        let tabs = TabBar::new("settings-sections")
            .segmented()
            .selected_index(
                sections
                    .iter()
                    .position(|section| *section == self.section)
                    .unwrap_or(0),
            )
            .children(sections.map(|section| Tab::new().label(section.name()).flex_1()))
            .on_click(cx.listener(move |view, ix: &usize, _, cx| {
                view.section = sections[*ix];
                view.update(|preferences| preferences.settings_page = *ix, cx);
                view.capture = None;
                view.error = None;
                cx.notify();
            }));
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, action: &Intent, window, cx| {
                if crate::system_action(action, window, cx) {
                    return;
                }
                match *action {
                    Intent::Close => window.remove_window(),
                    Intent::Quit => crate::quit(cx),
                    Intent::NewWindow => crate::new_window(cx, None),
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
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    if view.capture.take().is_none() {
                        window.remove_window();
                    }
                    cx.notify();
                    return;
                }
                if let Some(ix) = view.capture.take() {
                    cx.stop_propagation();
                    let key = &event.keystroke.key;
                    let preferences = &cx.global::<Desktop>().preferences;
                    if ((key.len() == 1 && key != " ")
                        || matches!(
                            key.as_str(),
                            "space" | "left" | "right" | "up" | "down" | "enter"
                        ))
                        && !event.keystroke.modifiers.platform
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.alt
                        && (ix == 3 || key != &preferences.turbo_key)
                        && !preferences
                            .keys
                            .iter()
                            .enumerate()
                            .any(|(other, binding)| other != ix && binding == key)
                    {
                        let key = key.clone();
                        view.update(
                            |p| {
                                if ix == 3 {
                                    p.turbo_key = key;
                                } else {
                                    p.keys[ix] = key;
                                }
                            },
                            cx,
                        );
                        let players: Vec<_> = cx
                            .global::<Desktop>()
                            .windows
                            .iter()
                            .filter_map(|w| w.view.upgrade())
                            .collect();
                        for player in players {
                            player.update(cx, |view, _| view.release());
                        }
                    } else {
                        view.error = Some("Choose a different key.");
                    }
                    cx.notify();
                }
            }))
            .child(theme::title("Settings", window, cx))
            .child(div().px_6().pt_4().pb_2().child(tabs))
            .child(
                div()
                    .id("settings-pane")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(v_flex().p_6().gap_5().child(content).when_some(
                        self.error,
                        |root, error| {
                            root.child(div().text_sm().text_color(cx.theme().danger).child(error))
                        },
                    )),
            )
    }
}
