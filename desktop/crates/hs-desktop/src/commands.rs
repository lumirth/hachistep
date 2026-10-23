use gpui_kit::*;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hachistep, no_json)]
pub(crate) enum Intent {
    NewWindow,
    Palette(Option<String>),
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    BringAll,
    OpenFirmware,
    OpenEeprom,
    NewEeprom,
    RecentEeprom(crate::settings::Recent),
    RecentFirmware(std::path::PathBuf),
    Info,
    ClearRecents,
    RevertEeprom,
    SaveEeprom,
    SaveEepromAs,
    SaveState,
    LoadState,
    SaveSelectedState,
    LoadSelectedState,
    SaveSlot(u8),
    LoadSlot(u8),
    SelectSlot(u8),
    UndoLoadState,
    ToggleTurbo,
    TurboRate(u16),
    Cadence(u16),
    Disconnect,
    CopyScreen,
    Larger,
    Smaller,
    EditPalettes,
    Screenshot,
    Close,
    Quit,
    Pause,
    ToggleWalking,
    Reset,
    Speed(u16),
    Mute,
    Scale(u8),
    Fit,
    Fullscreen,
    Status,
    Raw,
    Tool(crate::tools::Kind),
    Settings,
    Controls,
    Documentation,
    Report,
    About,
}

pub(crate) fn fixed_bindings() -> Vec<KeyBinding> {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let mut bindings: Vec<_> = [
        ("n", Intent::NewWindow),
        ("o", Intent::OpenEeprom),
        ("shift-o", Intent::OpenFirmware),
        ("s", Intent::SaveEeprom),
        ("shift-s", Intent::SaveEepromAs),
        (",", Intent::Settings),
        ("w", Intent::Close),
        ("q", Intent::Quit),
    ]
    .into_iter()
    .map(|(key, intent)| KeyBinding::new(&format!("{modifier}-{key}"), intent, None))
    .collect();
    for slot in 1..=10 {
        bindings.extend([
            KeyBinding::new(
                &format!("f{slot}"),
                Intent::LoadSlot(slot),
                Some("EmulatorCommands && !Input"),
            ),
            KeyBinding::new(
                &format!("shift-f{slot}"),
                Intent::SaveSlot(slot),
                Some("EmulatorCommands && !Input"),
            ),
        ]);
    }
    bindings.extend([
        KeyBinding::new(&format!("{modifier}-="), Intent::Larger, None),
        KeyBinding::new(&format!("{modifier}--"), Intent::Smaller, None),
        KeyBinding::new(&format!("{modifier}-/"), Intent::Status, None),
        KeyBinding::new(
            &format!("alt-{modifier}-i"),
            Intent::Tool(crate::tools::Kind::Infrared),
            None,
        ),
        KeyBinding::new(
            &format!("{modifier}-z"),
            Intent::UndoLoadState,
            Some("Player"),
        ),
    ]);
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-h", Intent::Hide, None),
            KeyBinding::new("alt-cmd-h", Intent::HideOthers, None),
            KeyBinding::new("cmd-m", Intent::Minimize, None),
            KeyBinding::new("ctrl-cmd-f", Intent::Fullscreen, None),
        ]);
    }
    bindings
}

pub(crate) fn bind(cx: &mut App) {
    let retained: Vec<_> = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|binding| !binding.action().as_any().is::<Intent>())
        .cloned()
        .collect();
    cx.clear_key_bindings();
    cx.bind_keys(retained);
    cx.bind_keys(fixed_bindings());
    let preferences = &cx.global::<crate::Desktop>().preferences;
    let bindings: Vec<_> = shortcuts()
        .into_iter()
        .filter_map(|shortcut| {
            let key = shortcut.key(preferences);
            (!key.is_empty()).then(|| {
                let context = if shortcut.intent == Intent::CopyScreen {
                    "Player"
                } else {
                    "EmulatorCommands && !Input"
                };
                KeyBinding::new(&key, shortcut.intent, Some(context))
            })
        })
        .collect();
    cx.bind_keys(bindings);
    cx.global_mut::<crate::Desktop>().menu = None;
}

pub(crate) fn init(cx: &mut App) {
    use gpui_kit::component::WindowExt;
    bind(cx);
    let actions: Vec<_> = shortcuts()
        .into_iter()
        .map(|shortcut| shortcut.intent)
        .chain((1..=10).flat_map(|slot| [Intent::LoadSlot(slot), Intent::SaveSlot(slot)]))
        .collect();
    cx.observe_keystrokes(move |event, window, cx| {
        if event.action.is_some()
            || event.keystroke.key_char.is_some()
            || !window.has_focused_input(cx)
        {
            return;
        }
        let blocked = cx.key_bindings().borrow().bindings().any(|binding| {
            binding
                .action()
                .as_any()
                .downcast_ref::<Intent>()
                .is_some_and(|action| actions.contains(action))
                && binding.match_keystrokes(std::slice::from_ref(&event.keystroke)) == Some(false)
        });
        if blocked {
            // AppKit falls back to menu equivalents after GPUI declines a scoped binding.
            // Observe after editing actions so ordinary field shortcuts still take precedence.
            cx.stop_propagation();
        }
    })
    .detach();
}

pub(crate) struct Shortcut {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub default: String,
    pub intent: Intent,
}
impl Shortcut {
    pub fn key(&self, preferences: &crate::settings::Preferences) -> String {
        preferences
            .hotkeys
            .get(self.id)
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }
}
pub(crate) fn shortcuts() -> Vec<Shortcut> {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    [
        (
            "pause",
            "System",
            "Pause",
            format!("{modifier}-p"),
            Intent::Pause,
        ),
        ("reset", "System", "Reset", String::new(), Intent::Reset),
        (
            "fast-forward",
            "System",
            "Fast-forward",
            String::new(),
            Intent::ToggleTurbo,
        ),
        (
            "walk",
            "System",
            "Walk",
            String::new(),
            Intent::ToggleWalking,
        ),
        (
            "quick-save",
            "States",
            "Quick save",
            format!("shift-{modifier}-l"),
            Intent::SaveSelectedState,
        ),
        (
            "quick-load",
            "States",
            "Quick load",
            format!("{modifier}-l"),
            Intent::LoadSelectedState,
        ),
        (
            "undo-load",
            "States",
            "Undo load state",
            "f12".into(),
            Intent::UndoLoadState,
        ),
        (
            "copy-screen",
            "Screen",
            "Copy screen",
            format!("{modifier}-c"),
            Intent::CopyScreen,
        ),
        (
            "screenshot",
            "Screen",
            "Save screenshot",
            String::new(),
            Intent::Screenshot,
        ),
    ]
    .into_iter()
    .map(|(id, group, label, default, intent)| Shortcut {
        id,
        group,
        label,
        default,
        intent,
    })
    .collect()
}

fn item(name: &str, intent: Intent) -> MenuItem {
    MenuItem::action(name.to_owned(), intent)
}

#[derive(Clone, PartialEq)]
pub(crate) struct MenuState {
    ready: bool,
    paused: bool,
    walking: bool,
    cadence: u16,
    connected: bool,
    speed_percent: u16,
    turbo_percent: u16,
    turbo: bool,
    muted: bool,
    modified: bool,
    saving: bool,
    can_undo: bool,
    slots: [Option<u64>; 10],
    has_eeprom: bool,
    raw: bool,
    status: bool,
    scale: u8,
    recent: Vec<crate::settings::Recent>,
    recent_firmware: Vec<std::path::PathBuf>,
    palettes: Vec<(String, String)>,
    palette_id: Option<String>,
    selected_slot: u8,
    bindings: Vec<String>,
}

impl MenuState {
    pub fn new(
        frame: &hs_runtime::Presentation,
        raw: bool,
        status: bool,
        scale: u8,
        preferences: &crate::settings::Preferences,
        selected_slot: u8,
    ) -> Self {
        Self {
            ready: frame.ready,
            paused: frame.paused,
            walking: frame.walking,
            cadence: frame.cadence,
            connected: frame.is_connection_active(),
            speed_percent: frame.speed_percent,
            turbo_percent: frame.turbo_percent,
            turbo: frame.turbo,
            muted: frame.muted,
            modified: frame.modified,
            saving: frame.saving,
            can_undo: frame.can_undo,
            slots: frame.slots,
            has_eeprom: frame.eeprom.is_some(),
            raw,
            status,
            scale,
            recent: preferences.recent.clone(),
            recent_firmware: preferences.recent_firmware.clone(),
            palettes: preferences
                .palettes
                .iter()
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect(),
            palette_id: None,
            selected_slot,
            bindings: shortcuts()
                .iter()
                .map(|shortcut| shortcut.key(preferences))
                .collect(),
        }
    }
    pub fn install(&self, cx: &mut App) {
        if cx.global::<crate::Desktop>().menu.as_ref() == Some(self) {
            return;
        }
        let menus = self.menus();
        fn collect(items: &[MenuItem], inherited: bool, disabled: &mut Vec<Intent>) {
            for item in items {
                match item {
                    MenuItem::Action {
                        action,
                        disabled: item_disabled,
                        ..
                    } if inherited || *item_disabled => {
                        if let Some(action) = action.as_any().downcast_ref::<Intent>() {
                            disabled.push(action.clone());
                        }
                    }
                    MenuItem::Submenu(menu) => {
                        collect(&menu.items, inherited || menu.disabled, disabled)
                    }
                    _ => (),
                }
            }
        }
        let mut disabled = Vec::new();
        for menu in &menus {
            collect(&menu.items, menu.disabled, &mut disabled);
        }
        cx.global_mut::<crate::Desktop>().disabled_commands = disabled;
        cx.global_mut::<crate::Desktop>().menu = Some(self.clone());
        cx.set_menus(menus);
        if let Some(menus) = cx.get_menus() {
            gpui_kit::base::GlobalState::global_mut(cx).set_app_menus(menus);
        }
    }

    fn active(&self, name: &str, intent: Intent) -> MenuItem {
        item(name, intent).disabled(!self.ready)
    }
    fn info(&self, name: &str) -> MenuItem {
        item(name, Intent::Info).disabled(true)
    }
    pub fn speeds(&self) -> Vec<MenuItem> {
        let mut items = Vec::new();
        if self.connected {
            items.push(self.info("100% while infrared is connected"));
            items.push(MenuItem::separator());
        }
        items.extend([50, 75, 100, 150, 200, 400, 800].map(|percent| {
            item(&format!("{percent}%"), Intent::Speed(percent))
                .checked(percent == self.speed_percent && !self.turbo)
                .disabled(!self.ready || (percent != 100 && self.connected))
        }));
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(Menu::new("Fast-forward Rate").items(
            [200, 400, 800].map(|percent| {
                self.active(&format!("{percent}%"), Intent::TurboRate(percent))
                    .checked(percent == self.turbo_percent)
            }),
        )));
        items
    }
    fn slot_label(&self, slot: u8) -> String {
        use chrono::{Local, TimeZone};
        let saved = self.slots[usize::from(slot - 1)];
        let suffix = match saved {
            None => "Empty".into(),
            Some(0) => "Saved".into(),
            Some(t) => Local
                .timestamp_opt(t as i64, 0)
                .single()
                .map(|date| date.format("%b %-d, %H:%M").to_string())
                .unwrap_or_else(|| "Saved".into()),
        };
        format!("Slot {slot} · {suffix}")
    }
    fn state_menu(&self, load: bool) -> MenuItem {
        let mut items = Vec::new();
        if !self.has_eeprom {
            items.push(self.info("Save the EEPROM to use slots"));
        } else {
            items.extend((1..=10).map(|slot| {
                self.active(
                    &self.slot_label(slot),
                    if load {
                        Intent::LoadSlot(slot)
                    } else {
                        Intent::SaveSlot(slot)
                    },
                )
                .disabled(!self.ready || (load && self.slots[usize::from(slot - 1)].is_none()))
            }));
        }
        items.push(MenuItem::separator());
        items.push(self.active(
            if load {
                "Load from File…"
            } else {
                "Save to File…"
            },
            if load {
                Intent::LoadState
            } else {
                Intent::SaveState
            },
        ));
        MenuItem::submenu(Menu::new(if load { "Load State" } else { "Save State" }).items(items))
    }
    fn quick_states(&self) -> Vec<MenuItem> {
        vec![
            self.active(
                &format!("Quick Save to Slot {}", self.selected_slot),
                Intent::SaveSelectedState,
            )
            .disabled(!self.ready || !self.has_eeprom),
            self.active(
                &format!("Quick Load from Slot {}", self.selected_slot),
                Intent::LoadSelectedState,
            )
            .disabled(!self.ready || self.slots[usize::from(self.selected_slot - 1)].is_none()),
            self.state_menu(false),
            self.state_menu(true),
            MenuItem::submenu(
                Menu::new("Quick Slot")
                    .disabled(!self.ready || !self.has_eeprom)
                    .items((1..=10).map(|slot| {
                        item(&format!("Slot {slot}"), Intent::SelectSlot(slot))
                            .checked(slot == self.selected_slot)
                    })),
            ),
            self.active("Undo Load State", Intent::UndoLoadState)
                .disabled(!self.ready || !self.can_undo),
        ]
    }
    fn scale_menu(&self) -> MenuItem {
        MenuItem::submenu(
            Menu::new("Display Scale").disabled(!self.ready).items(
                [
                    self.active("Larger", Intent::Larger),
                    self.active("Smaller", Intent::Smaller),
                    MenuItem::separator(),
                ]
                .into_iter()
                .chain((1..=8).map(|value| {
                    item(&format!("{value}×"), Intent::Scale(value)).checked(value == self.scale)
                })),
            ),
        )
    }
    fn pace_menu(&self) -> MenuItem {
        let mut items: Vec<_> = [80, 100, 120, 140, 160]
            .map(|pace| {
                self.active(&format!("{pace} steps/min"), Intent::Cadence(pace))
                    .checked(pace == self.cadence)
            })
            .into();
        if ![80, 100, 120, 140, 160].contains(&self.cadence) {
            items.push(self.info(&format!("Custom · {} steps/min", self.cadence)));
        }
        items.extend([
            MenuItem::separator(),
            self.active("Custom…", Intent::Tool(crate::tools::Kind::Motion)),
        ]);
        MenuItem::submenu(Menu::new("Walking Pace").disabled(!self.ready).items(items))
    }
    pub fn recents(&self) -> Vec<MenuItem> {
        let firmware: Vec<_> = self
            .recent_firmware
            .iter()
            .map(|path| path.as_path())
            .collect();
        let eeprom: Vec<_> = self
            .recent
            .iter()
            .map(|pair| pair.eeprom.as_path())
            .collect();
        let mut items: Vec<_> = self
            .recent
            .iter()
            .map(|pair| {
                item(
                    &crate::settings::recent_label(&pair.eeprom, &eeprom),
                    Intent::RecentEeprom(pair.clone()),
                )
            })
            .collect();
        if !items.is_empty() && !firmware.is_empty() {
            items.push(MenuItem::separator());
        }
        items.extend(self.recent_firmware.iter().map(|path| {
            item(
                &crate::settings::recent_label(path, &firmware),
                Intent::RecentFirmware(path.clone()),
            )
        }));
        if !items.is_empty() {
            items.extend([
                MenuItem::separator(),
                item("Clear Menu", Intent::ClearRecents),
            ]);
        }
        items
    }
    pub fn with_palette(mut self, id: Option<&str>) -> Self {
        self.palette_id = id.map(str::to_owned);
        self
    }
    fn palette_menu(&self) -> MenuItem {
        MenuItem::submenu(
            Menu::new("Palette").disabled(!self.ready).items(
                [
                    item("Use Default", Intent::Palette(None)).checked(self.palette_id.is_none()),
                    MenuItem::separator(),
                ]
                .into_iter()
                .chain(self.palettes.iter().map(|(id, name)| {
                    item(name, Intent::Palette(Some(id.clone())))
                        .checked(self.palette_id.as_ref() == Some(id))
                }))
                .chain([
                    MenuItem::separator(),
                    item("Edit Palettes…", Intent::EditPalettes),
                ]),
            ),
        )
    }
    fn playback(&self) -> Vec<MenuItem> {
        vec![
            self.active("Pause", Intent::Pause).checked(self.paused),
            self.active("Fast-forward", Intent::ToggleTurbo)
                .checked(self.turbo)
                .disabled(!self.ready || self.connected),
            MenuItem::submenu(
                Menu::new("Speed")
                    .disabled(!self.ready)
                    .items(self.speeds()),
            ),
        ]
    }
    fn inputs(&self) -> Vec<MenuItem> {
        vec![
            self.active("Walk", Intent::ToggleWalking)
                .checked(self.walking),
            self.pace_menu(),
            self.active("Infrared…", Intent::Tool(crate::tools::Kind::Infrared)),
            self.active("Disconnect", Intent::Disconnect)
                .disabled(!self.ready || !self.connected),
        ]
    }
    fn menus(&self) -> Vec<Menu> {
        use gpui_kit::component::input;
        let recent = self.recents();
        let mut files = vec![
            item("New Window", Intent::NewWindow),
            item("New EEPROM", Intent::NewEeprom),
            item("Open EEPROM…", Intent::OpenEeprom),
            item("Open Firmware…", Intent::OpenFirmware),
            MenuItem::submenu(
                Menu::new("Open Recent")
                    .disabled(recent.is_empty())
                    .items(recent),
            ),
            MenuItem::separator(),
            item("Close Window", Intent::Close),
            self.active("Save EEPROM", Intent::SaveEeprom)
                .disabled(!self.ready || !self.modified || self.saving),
            self.active("Save EEPROM As…", Intent::SaveEepromAs)
                .disabled(!self.ready || self.saving),
            self.active("Revert EEPROM to Saved…", Intent::RevertEeprom)
                .disabled(!self.ready || !self.has_eeprom || !self.modified || self.saving),
            MenuItem::separator(),
            self.active("Save Screenshot…", Intent::Screenshot),
        ];
        let mut tools = vec![
            self.active("Walking…", Intent::Tool(crate::tools::Kind::Motion)),
            self.active("Infrared…", Intent::Tool(crate::tools::Kind::Infrared)),
        ];
        if !cfg!(target_os = "macos") {
            tools.extend([MenuItem::separator(), item("Settings…", Intent::Settings)]);
            files.extend([MenuItem::separator(), item("Exit", Intent::Quit)]);
        }
        let mut system = vec![
            self.active("Pause", Intent::Pause).checked(self.paused),
            self.active("Reset", Intent::Reset),
            MenuItem::separator(),
        ];
        system.extend(self.playback().into_iter().skip(1));
        system.extend(self.inputs().into_iter().take(2));
        system.push(MenuItem::separator());
        system.extend(self.quick_states());
        system.extend([
            MenuItem::separator(),
            self.active("Mute", Intent::Mute).checked(self.muted),
        ]);
        let mut menus = vec![
            Menu::new("File").items(files),
            Menu::new("Edit").items([
                MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            ]),
            Menu::new("System").items(system),
            Menu::new("View").items([
                self.scale_menu(),
                self.active("Fit Window to Scale", Intent::Fit),
                self.palette_menu(),
                self.active("Apply LCD Contrast", Intent::Raw)
                    .checked(!self.raw),
                MenuItem::separator(),
                self.active(
                    if self.status {
                        "Hide Status Bar"
                    } else {
                        "Show Status Bar"
                    },
                    Intent::Status,
                ),
                item("Enter Full Screen", Intent::Fullscreen),
            ]),
            Menu::new("Tools").items(tools),
            Menu::new("Help").items([
                item("Documentation", Intent::Documentation),
                item("Controls…", Intent::Controls),
                MenuItem::separator(),
                item("Report a Problem", Intent::Report),
            ]),
        ];
        if cfg!(target_os = "macos") {
            menus.insert(
                0,
                Menu::new("hachistep").items([
                    item("About hachistep", Intent::About),
                    MenuItem::separator(),
                    item("Settings…", Intent::Settings),
                    MenuItem::separator(),
                    MenuItem::os_submenu("Services", SystemMenuType::Services),
                    MenuItem::separator(),
                    item("Hide hachistep", Intent::Hide),
                    item("Hide Others", Intent::HideOthers),
                    item("Show All", Intent::ShowAll),
                    MenuItem::separator(),
                    item("Quit hachistep", Intent::Quit),
                ]),
            );
            menus.insert(
                menus.len() - 1,
                Menu::new("Window").items([
                    item("Minimize", Intent::Minimize),
                    item("Zoom", Intent::Zoom),
                    MenuItem::separator(),
                    item("Bring All to Front", Intent::BringAll),
                ]),
            );
        } else {
            menus
                .last_mut()
                .unwrap()
                .items
                .push(item("About hachistep", Intent::About));
        }
        menus
    }
    pub fn context(&self) -> Vec<MenuItem> {
        let mut items = self.playback();
        items.push(MenuItem::separator());
        items.extend(self.quick_states());
        items.push(MenuItem::separator());
        items.extend(self.inputs());
        items.extend([
            self.active("Mute", Intent::Mute).checked(self.muted),
            MenuItem::separator(),
            self.active("Copy Screen", Intent::CopyScreen),
            self.active("Save Screenshot…", Intent::Screenshot),
            MenuItem::separator(),
            self.scale_menu(),
            self.palette_menu(),
            MenuItem::separator(),
            self.active("Reset", Intent::Reset),
        ]);
        items
    }
}

pub(crate) fn native(
    items: impl IntoIterator<Item = MenuItem>,
) -> gpui_kit::component::native_menu::NativeMenu {
    Menu::new("").items(items).into()
}
