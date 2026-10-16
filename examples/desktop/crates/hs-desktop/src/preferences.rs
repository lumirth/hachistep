use crate::{
    commands::{Intent, Invoke},
    settings::{self, Preferences},
    Desktop,
};
use gpui_kit::{
    component::{
        self,
        button::Button,
        checkbox::Checkbox,
        menu::{DropdownMenu, PopupMenuItem},
        radio::RadioGroup,
        sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
        ActiveTheme, Sizable,
    },
    prelude::*,
    *,
};

pub(crate) fn open(cx: &mut App) {
    if let Some(handle) = cx.global::<Desktop>().preferences_window {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let mut options = crate::options("Settings", 640., 420.);
    options.window_min_size = Some(size(px(540.), px(360.)));
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Settings::new(window, cx))
    }) {
        Ok((handle, _)) => cx.global_mut::<Desktop>().preferences_window = Some(handle),
        Err(error) => eprintln!("Could not open Settings: {error:#}"),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Appearance,
    Controls,
    Audio,
    Emulation,
}
impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Controls => "Controls",
            Self::Audio => "Audio",
            Self::Emulation => "Emulation",
        }
    }
}

struct Settings {
    section: Section,
    focus: FocusHandle,
    capture: Option<usize>,
    error: Option<&'static str>,
    _release: Subscription,
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let release = cx.on_release(|_, cx| cx.global_mut::<Desktop>().preferences_window = None);
        Self {
            section: Section::Appearance,
            focus,
            capture: None,
            error: None,
            _release: release,
        }
    }
    fn update(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        change(&mut cx.global_mut::<Desktop>().preferences);
        crate::save_preferences(cx);
        cx.notify();
    }
    fn appearance(&self, cx: &mut Context<Self>) -> AnyElement {
        RadioGroup::new("appearance")
            .children(["System", "Light", "Dark"])
            .selected_index(Some(cx.global::<Desktop>().preferences.appearance))
            .on_change(cx.listener(|view, value, window, cx| {
                view.update(|p| p.appearance = *value, cx);
                let mode = match value {
                    1 => component::ThemeMode::Light,
                    2 => component::ThemeMode::Dark,
                    _ => cx.window_appearance().into(),
                };
                component::Theme::change(mode, Some(window), cx);
            }))
            .into_any_element()
    }
    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let preferences = cx.global::<Desktop>().preferences.clone();
        let mut keyboard = div().flex().flex_col().gap_2().child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Keyboard"),
        );
        let mut gamepad = div().flex().flex_col().gap_2().child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Gamepad"),
        );
        for (ix, label) in ["Left", "Center", "Right"].into_iter().enumerate() {
            keyboard = keyboard.child(self.key_row(label, ix, &preferences.keys[ix], cx));
            let owner = cx.weak_entity();
            gamepad = gamepad.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(label)
                    .child(
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
                                                            settings::GAMEPAD[ix
                                                                .min(settings::GAMEPAD.len() - 1)]
                                                            .0
                                                        },
                                                    );
                                                cx.global::<Desktop>()
                                                    .runtime
                                                    .gamepad_mapping(mapping);
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
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(keyboard)
            .child(gamepad)
            .into_any_element()
    }
    fn key_row(&self, label: &str, index: usize, binding: &str, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(label.to_owned())
            .child(
                Button::new(("key", index))
                    .small()
                    .w_32()
                    .label(if self.capture == Some(index) {
                        "Press a key".into()
                    } else if binding == "space" {
                        "Space".into()
                    } else {
                        binding.to_uppercase()
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

    fn audio(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                Checkbox::new("mute-new")
                    .label("Mute new players")
                    .checked(cx.global::<Desktop>().preferences.mute)
                    .on_change(
                        cx.listener(|view, value, _, cx| view.update(|p| p.mute = *value, cx)),
                    ),
            )
            .child(
                Button::new("retry-audio")
                    .label("Reconnect Audio")
                    .on_click(|_, _, cx| cx.global::<Desktop>().runtime.retry_audio()),
            )
            .into_any_element()
    }
    fn emulation(&self, cx: &mut Context<Self>) -> AnyElement {
        let preferences = &cx.global::<Desktop>().preferences;
        div()
            .flex()
            .flex_col()
            .gap_4()
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
            ))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Audio is muted outside 100%."),
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
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(label)
            .child(
                Button::new(id)
                    .small()
                    .w_32()
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.section {
            Section::Appearance => self.appearance(cx),
            Section::Controls => self.controls(cx),
            Section::Audio => self.audio(cx),
            Section::Emulation => self.emulation(cx),
        };
        let menu = SidebarMenu::new().children(
            [
                Section::Appearance,
                Section::Controls,
                Section::Audio,
                Section::Emulation,
            ]
            .map(|section| {
                SidebarMenuItem::new(section.name())
                    .active(self.section == section)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.section = section;
                        view.capture = None;
                        view.error = None;
                        cx.notify();
                    }))
            }),
        );
        div()
            .size_full()
            .flex()
            .items_stretch()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|_, action: &Invoke, window, cx| match action.intent {
                    Intent::Close => window.remove_window(),
                    Intent::Quit => crate::quit(cx),
                    Intent::NewWindow => crate::new_window(cx, None),
                    _ => (),
                }),
            )
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
            .child(
                Sidebar::new("settings-sections")
                    .collapsible(false)
                    .w_40()
                    .h_full()
                    .child(menu),
            )
            .child(
                div()
                    .id("settings-pane")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .p_6()
                            .gap_5()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.section.name()),
                            )
                            .child(content)
                            .when_some(self.error, |root, error| {
                                root.child(
                                    div().text_sm().text_color(cx.theme().danger).child(error),
                                )
                            }),
                    ),
            )
    }
}
