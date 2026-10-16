use crate::{
    commands::Intent,
    lcd,
    settings::{self, Preferences},
    theme, Desktop,
};
use gpui_kit::{
    component::{
        button::Button,
        color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
        h_flex,
        input::{Input, InputEvent, InputState},
        kbd::Kbd,
        menu::{DropdownMenu, PopupMenuItem},
        radio::RadioGroup,
        switch::Switch,
        tab::{Tab, TabBar},
        v_flex, ActiveTheme, Sizable,
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
fn open_section(section: Option<Section>, cx: &mut App) {
    if let Some((handle, view)) = cx.global::<Desktop>().preferences_window.clone() {
        if handle
            .update(cx, |_, window, cx| {
                let _ = view.update(cx, |view, cx| {
                    if let Some(section) = section {
                        view.section = section;
                        window.resize(size(px(520.), px(section.height(view.preview.is_some()))));
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
    let mut options = crate::options("Settings", 520., section.height(preview(cx).is_some()));
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

fn preview(cx: &App) -> Option<hs_runtime::Player> {
    cx.global::<Desktop>()
        .windows
        .iter()
        .rev()
        .filter_map(|w| w.view.upgrade())
        .find_map(|view| {
            let view = view.read(cx);
            view.presentation().ready.then(|| view.player().clone())
        })
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Appearance,
    Controls,
    Emulation,
}
impl Section {
    fn height(self, has_preview: bool) -> f32 {
        match self {
            Self::Emulation => 260.,
            Self::Appearance if !has_preview => 300.,
            _ => 420.,
        }
    }
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
    colors: [Entity<ColorPickerState>; 2],
    hex: [Entity<InputState>; 2],
    preview: Option<hs_runtime::Player>,
    target: Option<WeakEntity<crate::player::PlayerView>>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    fn new(section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        focus.focus(window, cx);
        let release = cx.on_release(|_, cx| cx.global_mut::<Desktop>().preferences_window = None);
        let palette = cx.global::<Desktop>().preferences.palette;
        let colors = [palette.panel, palette.ink]
            .map(|color| cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(color))));
        let hex = [palette.panel, palette.ink].map(|color| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(format!("{color:06X}"))
                    .validate(|text, _| {
                        text.len() <= 6 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
            })
        });
        let mut subscriptions = vec![release];
        for (index, picker) in colors.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                picker,
                window,
                move |view, _, event: &ColorPickerEvent, window, cx| {
                    if let ColorPickerEvent::Change(Some(color)) = event {
                        let rgba = color.to_rgb();
                        let value = ((rgba.r * 255.).round() as u32) << 16
                            | ((rgba.g * 255.).round() as u32) << 8
                            | (rgba.b * 255.).round() as u32;
                        view.set_color(index, value, window, cx);
                    }
                },
            ));
            subscriptions.push(cx.subscribe_in(
                &hex[index],
                window,
                move |view, _, event: &InputEvent, window, cx| {
                    let text = view.hex[index].read(cx).value();
                    if matches!(event, InputEvent::Change) && text.len() == 6 {
                        if let Ok(value) = u32::from_str_radix(&text, 16) {
                            view.set_color(index, value, window, cx);
                        }
                    } else if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        let palette = cx.global::<Desktop>().preferences.palette;
                        let value = if index == 0 {
                            palette.panel
                        } else {
                            palette.ink
                        };
                        view.set_color(index, value, window, cx);
                    }
                },
            ));
        }
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
        let preview = preview(cx);
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
                        if view.section == Section::Appearance && view.preview.is_some() {
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
            section,
            focus,
            capture: None,
            error: None,
            colors,
            hex,
            preview,
            target,
            _subscriptions: subscriptions,
        }
    }
    fn update(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        change(&mut cx.global_mut::<Desktop>().preferences);
        crate::save_preferences(cx);
        cx.refresh_windows();
        cx.notify();
    }
    fn set_color(&mut self, index: usize, value: u32, window: &mut Window, cx: &mut Context<Self>) {
        let palette = cx.global::<Desktop>().preferences.palette;
        let old = if index == 0 {
            palette.panel
        } else {
            palette.ink
        };
        if value != old {
            self.update(
                |p| {
                    if index == 0 {
                        p.palette.panel = value
                    } else {
                        p.palette.ink = value
                    }
                },
                cx,
            );
        }
        let text = format!("{value:06X}");
        if self.hex[index].read(cx).value().as_str() != text {
            self.hex[index].update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.colors[index].update(cx, |picker, cx| picker.set_value(rgb(value), window, cx));
    }
    fn appearance(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.global::<Desktop>().preferences.palette;
        let name = lcd::PRESETS
            .iter()
            .find(|(_, p)| *p == palette)
            .map(|(name, _)| *name)
            .unwrap_or("Custom");
        let owner = cx.weak_entity();
        let frame = self.preview.as_ref().map(hs_runtime::Player::presentation);
        v_flex()
            .gap_6()
            .child(
                h_flex().justify_between().gap_4().child("Theme").child(
                    RadioGroup::horizontal("appearance")
                        .children(["System", "Light", "Dark"])
                        .selected_index(Some(cx.global::<Desktop>().preferences.appearance))
                        .on_change(cx.listener(|view, value, _, cx| {
                            view.update(|p| p.appearance = *value, cx);
                            theme::apply(*value, cx);
                        })),
                ),
            )
            .child(
                v_flex()
                    .gap_4()
                    .child(h_flex().justify_between().child("LCD colors").child(
                        Button::new("palette").small().label(name).dropdown_menu(
                            move |mut menu, _, _| {
                                for (preset, palette) in lcd::PRESETS {
                                    let owner = owner.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(preset)
                                            .checked(preset == name)
                                            .on_click(move |_, window, cx| {
                                                let _ = owner.update(cx, |view, cx| {
                                                    view.update(|p| p.palette = palette, cx);
                                                    for (index, color) in
                                                        [palette.panel, palette.ink]
                                                            .into_iter()
                                                            .enumerate()
                                                    {
                                                        view.set_color(index, color, window, cx);
                                                    }
                                                });
                                            }),
                                    );
                                }
                                menu
                            },
                        ),
                    ))
                    .when_some(frame, |root, frame| {
                        root.child(div().flex().justify_center().py_2().child(
                            div().w_48().h_32().child(lcd::display(
                                frame.pixels,
                                frame.contrast,
                                false,
                                palette,
                            )),
                        ))
                    })
                    .child(
                        div().flex().gap_5().children(
                            ["Panel", "Ink"]
                                .into_iter()
                                .enumerate()
                                .map(|(index, label)| {
                                    div()
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(div().text_sm().child(label))
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    ColorPicker::new(&self.colors[index])
                                                        .accessibility_label(format!(
                                                            "Choose {label} color"
                                                        ))
                                                        .small(),
                                                )
                                                .child(
                                                    div().flex_1().min_w_0().child(
                                                        Input::new(&self.hex[index])
                                                            .small()
                                                            .prefix(
                                                                div()
                                                                    .text_color(
                                                                        cx.theme().muted_foreground,
                                                                    )
                                                                    .child("#"),
                                                            ),
                                                    ),
                                                ),
                                        )
                                }),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .children([0, 85, 170, 255].map(|drive| {
                                div()
                                    .flex_1()
                                    .h_5()
                                    .bg(rgb(palette.colors(32, false)[drive]))
                            })),
                    ),
            )
            .into_any_element()
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
        v_flex()
            .gap_5()
            .child(
                Switch::new("sound")
                    .label("Sound")
                    .checked(!preferences.mute)
                    .on_change(cx.listener(|view, enabled: &bool, _, cx| {
                        view.update(|p| p.mute = !*enabled, cx);
                        cx.global::<Desktop>().runtime.sound_enabled(*enabled);
                    })),
            )
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
                                Button::new("retry-audio").small().label("Retry").on_click(
                                    |_, _, cx| cx.global::<Desktop>().runtime.retry_audio(),
                                ),
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
            .on_click(cx.listener(move |view, ix: &usize, window, cx| {
                view.section = sections[*ix];
                window.resize(size(
                    px(520.),
                    px(view.section.height(view.preview.is_some())),
                ));
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
            .on_action(
                cx.listener(|view, action: &Intent, window, cx| match *action {
                    Intent::Close => window.remove_window(),
                    Intent::Quit => crate::quit(cx),
                    Intent::NewWindow => crate::new_window(cx, None),
                    Intent::Settings => (),
                    _ => {
                        if let Some(target) = &view.target {
                            crate::invoke_player(target, action.clone(), cx);
                        }
                    }
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
