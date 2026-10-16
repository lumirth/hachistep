use crate::{
    lcd::{self, Palette},
    settings::SavedPalette,
    theme, Desktop,
};
use gpui_kit::{
    component::{
        button::Button,
        color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
        dialog::DialogButtonProps,
        h_flex,
        input::{Input, InputEvent, InputState, NumberInput},
        menu::{DropdownMenu, PopupMenuItem},
        radio::RadioGroup,
        select::{SearchableVec, Select, SelectEvent, SelectGroup, SelectItem, SelectState},
        tab::{Tab, TabBar},
        v_flex, ActiveTheme, Sizable, WindowExt,
    },
    prelude::*,
    *,
};

pub(crate) struct Editor {
    selection: Entity<SelectState<Choices>>,
    focus: FocusHandle,
    draft: Palette,
    middle: Option<[u32; 2]>,
    source: Option<String>,
    name: Entity<InputState>,
    margin: Entity<InputState>,
    colors: [Entity<ColorPickerState>; 4],
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}
#[derive(Clone)]
struct Choice {
    id: String,
    name: String,
    colors: Palette,
}
type Choices = SearchableVec<SelectGroup<Choice>>;
impl Choice {
    fn row(&self) -> AnyElement {
        h_flex()
            .id(SharedString::from(self.id.clone()))
            .role(Role::Label)
            .aria_label(self.name.clone())
            .gap_3()
            .child(swatches(self.colors).w_16().flex_shrink_0())
            .child(self.name.clone())
            .into_any_element()
    }
}
impl SelectItem for Choice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.name.clone().into()
    }
    fn value(&self) -> &String {
        &self.id
    }
    fn display_title(&self) -> Option<AnyElement> {
        Some(self.row())
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.row()
    }
}
fn choices(p: &crate::settings::Preferences) -> Choices {
    let builtin = lcd::PRESETS
        .map(|(name, colors)| Choice {
            id: name.into(),
            name: name.into(),
            colors,
        })
        .to_vec();
    let mut mine: Vec<_> = p
        .palettes
        .iter()
        .map(|p| Choice {
            id: p.id.clone(),
            name: p.name.clone(),
            colors: p.colors,
        })
        .collect();
    if let Some(colors) = p.custom_palette {
        mine.push(Choice {
            id: "custom".into(),
            name: "Custom".into(),
            colors,
        });
    }
    let mut groups = vec![SelectGroup::new("Built-in").items(builtin)];
    if !mine.is_empty() {
        groups.push(SelectGroup::new("My Palettes").items(mine));
    }
    SearchableVec::new(groups)
}
fn swatches(palette: Palette) -> Div {
    h_flex()
        .h_3()
        .rounded_sm()
        .overflow_hidden()
        .children(palette.shades().map(|c| div().h_full().flex_1().bg(rgb(c))))
}
fn parse(value: &str) -> anyhow::Result<(String, Palette)> {
    let parts: Vec<_> = value.trim().split('|').map(str::trim).collect();
    anyhow::ensure!(
        parts.len() == 5 && !parts[0].is_empty(),
        "Enter a name and four colors separated by |."
    );
    let colors: Vec<_> = parts[1..]
        .iter()
        .map(|s| {
            anyhow::ensure!(s.len() == 6, "Use six hexadecimal digits for each color.");
            Ok(u32::from_str_radix(s, 16)?)
        })
        .collect::<anyhow::Result<_>>()?;
    Ok((
        parts[0].to_string(),
        Palette {
            panel: colors[0],
            ink: colors[3],
            middle: Some([colors[1], colors[2]]),
        },
    ))
}
impl Editor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let p = &cx.global::<Desktop>().preferences;
        let draft = p.palette;
        let id = p.palette_id.clone();
        let items = choices(p);
        let selection = cx.new(|cx| {
            let mut state = SelectState::new(items, None, window, cx);
            state.set_selected_value(&id, window, cx);
            state
        });
        let colors = draft
            .shades()
            .map(|c| cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(c))));
        let margin = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(cx.global::<Desktop>().preferences.lcd_margin.to_string())
                .min(0.)
                .max(128.)
                .step(1.)
        });
        let mut subscriptions = vec![cx.subscribe_in(
            &margin,
            window,
            |view, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Ok(value @ 0..=128) = input.read(cx).value().parse::<u16>() {
                        cx.global_mut::<Desktop>().preferences.lcd_margin = value;
                        view.changed(window, cx);
                    }
                }
            },
        )];
        subscriptions.push(cx.subscribe_in(
            &selection,
            window,
            |view, _, event: &SelectEvent<Choices>, window, cx| {
                if let SelectEvent::Confirm(Some(id)) = event {
                    cx.global_mut::<Desktop>().preferences.select_palette(id);
                    view.changed(window, cx);
                }
            },
        ));
        for (ix, color) in colors.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                color,
                window,
                move |view, _, event: &ColorPickerEvent, _, cx| {
                    if let ColorPickerEvent::Change(Some(color)) = event {
                        let rgb = color.to_rgb();
                        let value = ((rgb.r * 255.).round() as u32) << 16
                            | ((rgb.g * 255.).round() as u32) << 8
                            | (rgb.b * 255.).round() as u32;
                        view.draft = view.draft.with_color(ix, value);
                        if view.draft.middle.is_some() {
                            view.middle = view.draft.middle;
                        }
                        cx.notify();
                    }
                },
            ));
        }
        Self {
            selection,
            focus: cx.focus_handle().tab_stop(true),
            draft,
            middle: draft.middle,
            source: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Custom")),
            colors,
            margin,
            error: None,
            _subscriptions: subscriptions,
        }
    }
    fn changed(&self, window: &mut Window, cx: &mut Context<Self>) {
        let p = &cx.global::<Desktop>().preferences;
        let id = p.palette_id.clone();
        let items = choices(p);
        self.selection.update(cx, |selection, cx| {
            selection.set_items(items, window, cx);
            selection.set_selected_value(&id, window, cx);
        });
        crate::save_preferences(cx);
        cx.refresh_windows();
        cx.notify();
    }
    fn sync(&self, window: &mut Window, cx: &mut Context<Self>) {
        for (state, color) in self.colors.iter().zip(self.draft.shades()) {
            state.update(cx, |state, cx| {
                state.set_value(rgb(color), window, cx);
                state.sync_pending_value(window, cx);
                state.hex_input().update(cx, |input, cx| {
                    input.set_value(format!("#{color:06X}"), window, cx)
                });
            });
        }
    }
    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preferences = &cx.global::<Desktop>().preferences;
        self.draft = preferences.palette;
        self.middle = self.draft.middle;
        let saved = preferences
            .palettes
            .iter()
            .find(|p| p.id == preferences.palette_id);
        self.source = saved.map(|p| p.id.clone());
        let name = saved.map(|p| p.name.clone()).unwrap_or_default();
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.error = None;
        self.sync(window, cx);
        self.focus.focus(window, cx);
        let owner = cx.entity();
        let form = cx.new(|cx| Form {
            owner: owner.clone(),
            _subscription: cx.observe(&owner, |_, _, cx| cx.notify()),
        });
        window.open_alert_dialog(cx, move |dialog, window, _| {
            let owner = owner.clone();
            dialog
                .title("Edit Palette")
                .width(window.rem_size() * 26.)
                .child(form.clone())
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Save"))
                .on_ok(move |_, window, cx| owner.update(cx, |view, cx| view.save(window, cx)))
        });
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().trim().to_string();
        if name.contains(['|', '\n', '\r']) {
            self.error = Some("Choose a name without | or line breaks.".into());
            cx.notify();
            return false;
        }
        let p = &mut cx.global_mut::<Desktop>().preferences;
        if !name.is_empty()
            && p.palettes
                .iter()
                .any(|saved| saved.name == name && self.source.as_ref() != Some(&saved.id))
        {
            self.error = Some("A palette with this name already exists.".into());
            cx.notify();
            return false;
        }
        if name.is_empty() {
            p.edit_palette(self.draft);
        } else {
            let id = self.source.clone().unwrap_or_else(|| {
                format!(
                    "user-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                )
            });
            if let Some(saved) = p.palettes.iter_mut().find(|saved| saved.id == id) {
                saved.name = name;
                saved.colors = self.draft;
            } else {
                p.palettes.push(SavedPalette {
                    id: id.clone(),
                    name,
                    colors: self.draft,
                });
            }
            p.select_palette(&id);
        }
        self.focus.focus(window, cx);
        self.changed(window, cx);
        true
    }
    fn import(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Name|FFFFFF|AAAAAA|555555|000000"));
        let error = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let owner = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let field = input.clone();
            let owner = owner.clone();
            let failure = error.clone();
            dialog
                .title("Import Palette")
                .child(Input::new(&input))
                .when_some(error.borrow().clone(), |dialog, error| {
                    dialog.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Import"))
                .on_ok(move |_, window, cx| {
                    let result = parse(&field.read(cx).value());
                    match result {
                        Ok((name, colors)) => {
                            let _ = owner.update(cx, |view, cx| {
                                let p = &mut cx.global_mut::<Desktop>().preferences;
                                let id = format!(
                                    "user-{}",
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_nanos()
                                );
                                p.palettes.push(SavedPalette {
                                    id: id.clone(),
                                    name,
                                    colors,
                                });
                                p.select_palette(&id);
                                view.changed(window, cx);
                            });
                            true
                        }
                        Err(error) => {
                            *failure.borrow_mut() = Some(error.to_string());
                            cx.refresh_windows();
                            false
                        }
                    }
                })
        });
    }
    fn delete(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let p = &cx.global::<Desktop>().preferences;
        let Some(saved) = p
            .palettes
            .iter()
            .find(|saved| saved.id == p.palette_id)
            .cloned()
        else {
            return;
        };
        let owner = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let saved = saved.clone();
            dialog
                .title(format!("Delete \"{}\"?", saved.name))
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Delete"))
                .on_ok(move |_, window, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        let p = &mut cx.global_mut::<Desktop>().preferences;
                        p.palettes.retain(|p| p.id != saved.id);
                        p.edit_palette(p.palette);
                        view.changed(window, cx);
                    });
                    true
                })
        });
    }
    fn appearance(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = cx.global::<Desktop>().preferences.clone();
        let owner = cx.weak_entity();
        let saved = p.palettes.iter().any(|saved| saved.id == p.palette_id);
        v_flex()
            .gap_5()
            .child(
                h_flex().justify_between().gap_4().child("Theme").child(
                    RadioGroup::horizontal("appearance")
                        .children(["System", "Light", "Dark"])
                        .selected_index(Some(p.appearance))
                        .on_change(cx.listener(|view, value, window, cx| {
                            cx.global_mut::<Desktop>().preferences.appearance = *value;
                            theme::apply(*value, cx);
                            view.changed(window, cx);
                        })),
                ),
            )
            .child(component::separator::Separator::horizontal())
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child("LCD margin")
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().w_24().child(NumberInput::new(&self.margin).small()))
                            .child(div().text_sm().child("px")),
                    ),
            )
            .child(
                v_flex().gap_2().child("Palette").child(
                    h_flex()
                        .gap_2()
                        .child(
                            Select::new(&self.selection)
                                .small()
                                .flex_1()
                                .accessibility_label("Palette")
                                .menu_max_h(
                                    (window.viewport_size().height - component::TITLE_BAR_HEIGHT)
                                        / 2.
                                        - window.rem_size() * 2.,
                                ),
                        )
                        .child(
                            Button::new("edit-palette")
                                .small()
                                .label("Edit…")
                                .on_click(cx.listener(|view, _, window, cx| view.edit(window, cx))),
                        )
                        .child(
                            Button::new("palette-actions")
                                .small()
                                .icon(component::IconName::Ellipsis)
                                .accessibility_label("Palette actions")
                                .dropdown_menu(move |mut menu, _, _| {
                                    let import_owner = owner.clone();
                                    menu =
                                        menu.item(PopupMenuItem::new("Import Palette…").on_click(
                                            move |_, window, cx| {
                                                let _ = import_owner
                                                    .update(cx, |view, cx| view.import(window, cx));
                                            },
                                        ));
                                    if saved {
                                        let owner = owner.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new("Delete Palette…").on_click(
                                                move |_, window, cx| {
                                                    let _ = owner.update(cx, |view, cx| {
                                                        view.delete(window, cx)
                                                    });
                                                },
                                            ),
                                        );
                                    }
                                    menu.item(PopupMenuItem::new("Copy Palette").on_click(
                                        |_, _, cx| {
                                            let p = &cx.global::<Desktop>().preferences;
                                            let name = p
                                                .palettes
                                                .iter()
                                                .find(|saved| saved.id == p.palette_id)
                                                .map(|p| p.name.as_str())
                                                .unwrap_or(&p.palette_id);
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                format!(
                                                    "{name}|{}",
                                                    p.palette
                                                        .shades()
                                                        .map(|v| format!("{v:06X}"))
                                                        .join("|")
                                                ),
                                            ));
                                        },
                                    ))
                                }),
                        ),
                ),
            )
            .into_any_element()
    }
    fn form(&self, cx: &mut Context<Self>) -> AnyElement {
        let four = self.draft.middle.is_some();
        let labels = [
            ("Background", 0),
            ("Light shade", 1),
            ("Dark shade", 2),
            ("Foreground", 3),
        ];
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_4()
                    .child("Name")
                    .child(Input::new(&self.name).flex_1()),
            )
            .child(
                TabBar::new("palette-color-mode")
                    .segmented()
                    .selected_index(usize::from(four))
                    .children([
                        Tab::new().label("Two colors").flex_1(),
                        Tab::new().label("Four colors").flex_1(),
                    ])
                    .on_click(cx.listener(|view, ix: &usize, window, cx| {
                        if *ix == 0 {
                            view.middle = view.draft.middle;
                            view.draft.middle = None;
                        } else {
                            let shades = view.draft.shades();
                            view.draft.middle = Some(view.middle.unwrap_or([shades[1], shades[2]]));
                        }
                        view.sync(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                div().grid().grid_cols(2).gap_4().children(
                    labels
                        .into_iter()
                        .filter(|(_, ix)| four || *ix == 0 || *ix == 3)
                        .map(|(label, ix)| {
                            v_flex().items_start().gap_2().child(label).child(
                                ColorPicker::new(&self.colors[ix])
                                    .label(format!("#{:06X}", self.draft.shades()[ix]))
                                    .small()
                                    .accessibility_label(format!("{label} color"))
                                    .featured_colors(
                                        self.draft.shades().map(rgb).map(Into::into).to_vec(),
                                    ),
                            )
                        }),
                ),
            )
            .child(swatches(self.draft))
            .when_some(self.error.clone(), |root, error| {
                root.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .into_any_element()
    }
}
impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("palette-editor")
            .track_focus(&self.focus)
            .child(self.appearance(window, cx))
    }
}

struct Form {
    owner: Entity<Editor>,
    _subscription: Subscription,
}
impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner.update(cx, |view, cx| view.form(cx))
    }
}
