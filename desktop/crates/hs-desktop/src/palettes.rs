use crate::{
    lcd::{self, Palette},
    settings::SavedPalette,
    theme, Desktop,
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
        dialog::DialogButtonProps,
        h_flex,
        input::{Input, InputEvent, InputState, NumberInput, Textarea, TextareaState},
        menu::{DropdownMenu, PopupMenuItem},
        radio::RadioGroup,
        switch::Switch,
        tab::{Tab, TabBar},
        v_flex, ActiveTheme, Disableable, Selectable, Sizable, WindowExt,
    },
    prelude::*,
    *,
};

pub(crate) struct Editor {
    focus: FocusHandle,
    draft: Palette,
    middle: Option<[u32; 2]>,
    source: Option<String>,
    preview: Option<WeakEntity<crate::player::PlayerView>>,
    preview_raw: bool,
    name: Entity<InputState>,
    margin: Entity<InputState>,
    colors: [Entity<ColorPickerState>; 4],
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
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
fn parse_many(value: &str) -> anyhow::Result<Vec<(String, Palette)>> {
    let mut palettes = Vec::new();
    for (line, record) in value.lines().enumerate() {
        if record.trim().is_empty() {
            continue;
        }
        palettes
            .push(parse(record).map_err(|error| anyhow::anyhow!("Line {}: {error}", line + 1))?);
    }
    anyhow::ensure!(!palettes.is_empty(), "Enter at least one palette.");
    Ok(palettes)
}
fn export(palette: &SavedPalette) -> String {
    format!(
        "{}|{}",
        palette.name,
        palette
            .colors
            .shades()
            .map(|v| format!("{v:06X}"))
            .join("|")
    )
}
impl Editor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let p = &cx.global::<Desktop>().preferences;
        let draft = p.palette;
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
                        if let Some(id) = &view.source {
                            cx.global_mut::<Desktop>().palette_preview =
                                Some((id.clone(), view.draft));
                            cx.refresh_windows();
                        }
                        cx.notify();
                    }
                },
            ));
        }
        Self {
            focus: cx.focus_handle().tab_stop(true),
            draft,
            middle: draft.middle,
            source: None,
            preview: None,
            preview_raw: false,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Custom")),
            colors,
            margin,
            error: None,
            _subscriptions: subscriptions,
        }
    }
    fn changed(&self, _window: &mut Window, cx: &mut Context<Self>) {
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
    fn edit(&mut self, new: bool, window: &mut Window, cx: &mut Context<Self>) {
        let preferences = &cx.global::<Desktop>().preferences;
        self.draft = preferences.palette;
        self.middle = self.draft.middle;
        let saved = preferences
            .palettes
            .iter()
            .find(|p| p.id == preferences.palette_id);
        self.source = if new {
            None
        } else {
            saved.map(|p| p.id.clone())
        };
        let name = if new {
            String::new()
        } else {
            saved
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "Custom".into())
        };
        let desktop = cx.global::<Desktop>();
        let active = desktop.active_player.as_ref().map(|view| view.entity_id());
        let palette = self.source.as_deref().unwrap_or(&preferences.palette_id);
        let preview = desktop
            .windows
            .iter()
            .filter_map(|window| window.view.upgrade())
            .filter(|view| view.read(cx).presentation().ready)
            .max_by_key(|view| {
                let walker = view.read(cx);
                let selected = walker
                    .palette_id()
                    .filter(|id| preferences.palette_named(id).is_some())
                    .unwrap_or(&preferences.palette_id);
                (selected == palette, active == Some(view.entity_id()))
            });
        self.preview_raw = preview
            .as_ref()
            .is_some_and(|view| view.read(cx).raw_display());
        self.preview = preview.as_ref().map(Entity::downgrade);
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.error = None;
        self.sync(window, cx);
        self.focus.focus(window, cx);
        cx.global_mut::<Desktop>().palette_preview = self.source.clone().map(|id| (id, self.draft));
        let owner = cx.entity();
        let form = cx.new(|cx| Form {
            owner: owner.clone(),
            _subscription: cx.observe(&owner, |_, _, cx| cx.notify()),
        });
        window.open_alert_dialog(cx, move |dialog, window, _| {
            let owner = owner.clone();
            dialog
                .title(if new { "New Palette" } else { "Edit Palette" })
                .width(window.rem_size() * 26.)
                .child(form.clone())
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Save"))
                .on_ok(move |_, window, cx| owner.update(cx, |view, cx| view.save(window, cx)))
                .on_close(|_, _, cx| {
                    cx.global_mut::<Desktop>().palette_preview = None;
                    cx.refresh_windows();
                })
        });
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("Enter a palette name.".into());
            cx.notify();
            return false;
        }
        if name.contains(['|', '\n', '\r']) {
            self.error = Some("Choose a name without | or line breaks.".into());
            cx.notify();
            return false;
        }
        let p = &mut cx.global_mut::<Desktop>().preferences;
        let same_name = p
            .palettes
            .iter()
            .any(|saved| self.source.as_ref() == Some(&saved.id) && saved.name == name);
        if !same_name && p.palettes.iter().any(|saved| saved.name == name) {
            self.error = Some("A palette with this name already exists.".into());
            cx.notify();
            return false;
        }
        let id = if let Some(id) = &self.source {
            if let Some(saved) = p.palettes.iter_mut().find(|saved| saved.id == *id) {
                saved.name = name;
                saved.colors = self.draft;
            }
            id.clone()
        } else {
            p.add_palette(name, self.draft)
        };
        p.select_palette(&id);
        self.focus.focus(window, cx);
        self.changed(window, cx);
        true
    }
    fn import(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .placeholder("Name|FFFFFF|AAAAAA|555555|000000")
        });
        let error = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let owner = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, window, cx| {
            let field = input.clone();
            let owner = owner.clone();
            let failure = error.clone();
            dialog
                .title("Import Palettes")
                .width(window.rem_size() * 28.)
                .child(Textarea::new(&input).aria_label("Palette records"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("One palette per line."),
                )
                .when_some(error.borrow().clone(), |dialog, error| {
                    dialog.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Import"))
                .on_ok(
                    move |_, window, cx| match parse_many(&field.read(cx).value()) {
                        Ok(records) => {
                            let _ = owner.update(cx, |view, cx| {
                                let p = &mut cx.global_mut::<Desktop>().preferences;
                                let mut first = None;
                                for (name, colors) in records {
                                    let id = p.add_palette(name, colors);
                                    first.get_or_insert(id);
                                }
                                if let Some(id) = first {
                                    p.select_palette(&id);
                                }
                                view.changed(window, cx);
                            });
                            true
                        }
                        Err(error) => {
                            *failure.borrow_mut() = Some(error.to_string());
                            cx.refresh_windows();
                            false
                        }
                    },
                )
        });
    }
    fn delete(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let p = &cx.global::<Desktop>().preferences;
        if p.palettes.len() <= 1 {
            return;
        }
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
                        if p.palettes.len() <= 1 {
                            return;
                        }
                        let position = p.palettes.iter().position(|palette| palette.id == saved.id);
                        p.palettes.retain(|p| p.id != saved.id);
                        let fallback = p.palettes[position.unwrap_or(0).min(p.palettes.len() - 1)]
                            .id
                            .clone();
                        p.select_palette(&fallback);
                        view.changed(window, cx);
                    });
                    true
                })
        });
    }
    fn appearance(&self, _: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = cx.global::<Desktop>().preferences.clone();
        let saved = p.palettes.iter().any(|saved| saved.id == p.palette_id);
        let palettes = p.palettes.clone();
        v_flex()
            .gap_5()
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child("Appearance")
                    .child(
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
                v_flex()
                    .gap_2()
                    .child("Default palette")
                    .child(
                        v_flex()
                            .id("palette-list")
                            .max_h_48()
                            .overflow_y_scroll()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_md()
                            .children(palettes.into_iter().map(|palette| {
                                let SavedPalette { id, name, colors } = palette;
                                let selected = p.palette_id == id;
                                Button::new(SharedString::from(format!("palette-{id}")))
                                    .ghost()
                                    .small()
                                    .accessibility_label(name.clone())
                                    .w_full()
                                    .justify_start()
                                    .selected(selected)
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap_3()
                                            .child(swatches(colors).w_16().flex_shrink_0())
                                            .child(div().flex_1().min_w_0().truncate().child(name))
                                            .when(selected, |row| {
                                                row.child(
                                                    component::Icon::new(
                                                        gpui_kit::assets::IconName::Check,
                                                    )
                                                    .xsmall(),
                                                )
                                            }),
                                    )
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        cx.global_mut::<Desktop>().preferences.select_palette(&id);
                                        view.changed(window, cx);
                                    }))
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(Button::new("new-palette").small().label("New…").on_click(
                                cx.listener(|view, _, window, cx| view.edit(true, window, cx)),
                            ))
                            .child(
                                Button::new("edit-palette")
                                    .small()
                                    .label("Edit…")
                                    .disabled(!saved)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.edit(false, window, cx)
                                    })),
                            )
                            .child(div().flex_1())
                            .child(self.actions(cx)),
                    ),
            )
            .into_any_element()
    }
    fn actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        Button::new("palette-actions")
            .small()
            .label("More")
            .accessibility_label("Palette actions")
            .dropdown_menu(move |menu, _, cx| {
                let p = &cx.global::<Desktop>().preferences;
                let selected = p.palettes.iter().any(|palette| palette.id == p.palette_id);
                let last = p.palettes.len() <= 1;
                let defaults_changed = p.default_palettes_changed();
                let duplicate = owner.clone();
                let copy = owner.clone();
                let copy_all = owner.clone();
                let import = owner.clone();
                let delete = owner.clone();
                let restore = owner.clone();
                menu.item(
                    PopupMenuItem::new("Duplicate")
                        .disabled(!selected)
                        .on_click(move |_, window, cx| {
                            let _ = duplicate.update(cx, |view, cx| view.duplicate(window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Delete…")
                        .disabled(!selected || last)
                        .on_click(move |_, window, cx| {
                            let _ = delete.update(cx, |view, cx| view.delete(window, cx));
                        }),
                )
                .when(last, |menu| {
                    menu.item(PopupMenuItem::label("Keep at least one palette."))
                })
                .separator()
                .item(
                    PopupMenuItem::new("Import Palettes…").on_click(move |_, window, cx| {
                        let _ = import.update(cx, |view, cx| view.import(window, cx));
                    }),
                )
                .item(
                    PopupMenuItem::new("Copy Palette")
                        .disabled(!selected)
                        .on_click(move |_, _, cx| {
                            let _ = copy.update(cx, |view, cx| view.copy(false, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Copy All Palettes").on_click(move |_, _, cx| {
                        let _ = copy_all.update(cx, |view, cx| view.copy(true, cx));
                    }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Restore Default Palettes…")
                        .disabled(!defaults_changed)
                        .on_click(move |_, window, cx| {
                            let _ =
                                restore.update(cx, |view, cx| view.restore_defaults(window, cx));
                        }),
                )
            })
    }
    fn copy(&self, all: bool, cx: &mut Context<Self>) {
        let p = &cx.global::<Desktop>().preferences;
        let records = p
            .palettes
            .iter()
            .filter(|palette| all || palette.id == p.palette_id)
            .map(export)
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(records));
    }
    fn restore_defaults(&self, window: &mut Window, cx: &mut Context<Self>) {
        let p = &cx.global::<Desktop>().preferences;
        let modified = p.palettes.iter().any(|palette| {
            lcd::PRESETS.iter().any(|(id, colors)| {
                palette.id == *id && (palette.name != *id || palette.colors != *colors)
            })
        });
        if !modified {
            cx.global_mut::<Desktop>()
                .preferences
                .restore_default_palettes();
            self.changed(window, cx);
            return;
        }
        let owner = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            dialog
                .title("Restore Default Palettes?")
                .child("Your changes to the included palettes will be lost.")
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Restore"))
                .on_ok(move |_, window, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        cx.global_mut::<Desktop>()
                            .preferences
                            .restore_default_palettes();
                        view.changed(window, cx);
                    });
                    true
                })
        });
    }
    fn duplicate(&self, window: &mut Window, cx: &mut Context<Self>) {
        let p = &mut cx.global_mut::<Desktop>().preferences;
        if let Some(original) = p
            .palettes
            .iter()
            .find(|palette| palette.id == p.palette_id)
            .cloned()
        {
            let id = p.add_palette(format!("{} copy", original.name), original.colors);
            p.select_palette(&id);
            self.changed(window, cx);
        }
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
                        if let Some(id) = &view.source {
                            cx.global_mut::<Desktop>().palette_preview =
                                Some((id.clone(), view.draft));
                            cx.refresh_windows();
                        }
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
            .when_some(
                self.preview
                    .as_ref()
                    .and_then(WeakEntity::upgrade)
                    .and_then(|view| {
                        let frame = view.read(cx).presentation().clone();
                        frame.ready.then_some(frame)
                    }),
                |root, frame| {
                    root.child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex().justify_between().gap_3().child("Preview").child(
                                    h_flex()
                                        .gap_2()
                                        .child(div().text_sm().child("LCD contrast"))
                                        .child(
                                            Switch::new("preview-contrast")
                                                .small()
                                                .checked(!self.preview_raw)
                                                .accessibility_label(
                                                    "Apply LCD contrast to preview",
                                                )
                                                .on_change(cx.listener(
                                                    |view, contrast: &bool, _, cx| {
                                                        view.preview_raw = !*contrast;
                                                        cx.notify();
                                                    },
                                                )),
                                        ),
                                ),
                            )
                            .child(div().w_48().h_32().self_center().child(lcd::display(
                                frame.pixels,
                                frame.contrast,
                                self.preview_raw,
                                self.draft,
                            ))),
                    )
                },
            )
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
