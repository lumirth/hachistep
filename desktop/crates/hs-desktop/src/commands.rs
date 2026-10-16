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
    Recents,
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
    Screenshot,
    Close,
    Quit,
    Pause,
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

pub(crate) fn bind(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    for (key, intent) in [
        ("n", Intent::NewWindow),
        ("o", Intent::OpenFirmware),
        ("shift-o", Intent::OpenEeprom),
        ("s", Intent::SaveEeprom),
        ("l", Intent::LoadSelectedState),
        ("shift-s", Intent::SaveEepromAs),
        ("shift-l", Intent::LoadState),
        ("p", Intent::Pause),
        (",", Intent::Settings),
        ("w", Intent::Close),
        ("q", Intent::Quit),
    ] {
        cx.bind_keys([KeyBinding::new(&format!("{modifier}-{key}"), intent, None)]);
    }
    for slot in 1..=10 {
        cx.bind_keys([
            KeyBinding::new(&format!("f{slot}"), Intent::LoadSlot(slot), None),
            KeyBinding::new(&format!("shift-f{slot}"), Intent::SaveSlot(slot), None),
        ]);
    }
    cx.bind_keys([KeyBinding::new("f12", Intent::UndoLoadState, None)]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-h", Intent::Hide, None),
            KeyBinding::new("alt-cmd-h", Intent::HideOthers, None),
            KeyBinding::new("cmd-m", Intent::Minimize, None),
        ]);
    }
}

fn item(name: &str, intent: Intent) -> MenuItem {
    MenuItem::action(name.to_owned(), intent)
}

#[derive(PartialEq)]
pub(crate) struct MenuState {
    ready: bool,
    paused: bool,
    connected: bool,
    speed_percent: u16,
    turbo: bool,
    muted: bool,
    modified: bool,
    saving: bool,
    can_undo: bool,
    slots: [bool; 10],
    has_eeprom: bool,
    raw: bool,
    status: bool,
    scale: u8,
    recent: Vec<crate::settings::Recent>,
    recent_firmware: Vec<std::path::PathBuf>,
    palettes: Vec<(String, String)>,
    palette_id: Option<String>,
    selected_slot: u8,
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
            connected: frame.is_connection_active(),
            speed_percent: frame.speed_percent,
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
            palettes: crate::lcd::PRESETS
                .iter()
                .map(|(name, _)| (name.to_string(), name.to_string()))
                .chain(
                    preferences
                        .palettes
                        .iter()
                        .map(|p| (p.id.clone(), p.name.clone())),
                )
                .chain(
                    preferences
                        .custom_palette
                        .map(|_| ("custom".into(), "Custom".into())),
                )
                .collect(),
            palette_id: None,
            selected_slot,
        }
    }

    pub fn install(self, cx: &mut App) {
        if cx.global::<crate::Desktop>().menu.as_ref() == Some(&self) {
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
        cx.global_mut::<crate::Desktop>().menu = Some(self);
        cx.set_menus(menus);
    }

    fn active(&self, name: &str, intent: Intent) -> MenuItem {
        item(name, intent).disabled(!self.ready)
    }

    pub fn speeds(&self) -> [MenuItem; 7] {
        [50, 75, 100, 150, 200, 400, 800].map(|percent| {
            item(&format!("{percent}%"), Intent::Speed(percent))
                .checked(percent == self.speed_percent)
                .disabled(!self.ready || (percent != 100 && self.connected))
        })
    }

    fn playback(&self) -> [MenuItem; 2] {
        [
            self.active(if self.paused { "Resume" } else { "Pause" }, Intent::Pause),
            self.active("Reset", Intent::Reset),
        ]
    }

    fn load_menu(&self) -> MenuItem {
        MenuItem::submenu(
            Menu::new("Load State").items(
                std::iter::once(
                    self.active("Load from Selected Slot", Intent::LoadSelectedState)
                        .disabled(!self.ready || !self.slots[usize::from(self.selected_slot - 1)]),
                )
                .chain([MenuItem::separator()])
                .chain((1..=10).map(|slot| {
                    item(&format!("Slot {slot}"), Intent::LoadSlot(slot))
                        .checked(slot == self.selected_slot)
                        .disabled(!self.ready || !self.slots[usize::from(slot - 1)])
                }))
                .chain([
                    MenuItem::separator(),
                    self.active("Load from File…", Intent::LoadState),
                ]),
            ),
        )
    }

    fn save_menu(&self) -> MenuItem {
        MenuItem::submenu(
            Menu::new("Save State").items(
                std::iter::once(
                    self.active("Save to Selected Slot", Intent::SaveSelectedState)
                        .disabled(!self.has_eeprom),
                )
                .chain([MenuItem::separator()])
                .chain((1..=10).map(|slot| {
                    self.active(&format!("Slot {slot}"), Intent::SaveSlot(slot))
                        .disabled(!self.has_eeprom)
                        .checked(slot == self.selected_slot)
                }))
                .chain([
                    MenuItem::separator(),
                    self.active("Save to File…", Intent::SaveState),
                ]),
            ),
        )
    }

    fn scale_menu(&self) -> MenuItem {
        MenuItem::submenu(Menu::new("Display Scale").items([4, 5, 6, 8].map(|value| {
            item(&format!("{value}×"), Intent::Scale(value)).checked(value == self.scale)
        })))
    }

    fn infrared_menu(&self) -> MenuItem {
        MenuItem::submenu(Menu::new("Infrared").items([
            self.active("Local Link…", Intent::Tool(crate::tools::Kind::Link)),
            self.active("TCP Connection…", Intent::Tool(crate::tools::Kind::Tcp)),
        ]))
    }

    pub fn recents(&self) -> [MenuItem; 2] {
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
        [
            MenuItem::submenu(
                Menu::new("Recent Firmware").items(self.recent_firmware.iter().map(|path| {
                    item(
                        &crate::settings::recent_label(path, &firmware),
                        Intent::RecentFirmware(path.clone()),
                    )
                })),
            ),
            MenuItem::submenu(
                Menu::new("Recent EEPROM").items(self.recent.iter().map(|pair| {
                    item(
                        &crate::settings::recent_label(&pair.eeprom, &eeprom),
                        Intent::RecentEeprom(pair.clone()),
                    )
                })),
            ),
        ]
    }

    pub fn with_palette(mut self, id: Option<&str>) -> Self {
        self.palette_id = id.map(str::to_owned);
        self
    }
    fn palette_menu(&self) -> MenuItem {
        MenuItem::submenu(
            Menu::new("Palette").items(
                std::iter::once(
                    item("Use Default", Intent::Palette(None)).checked(self.palette_id.is_none()),
                )
                .chain(std::iter::once(MenuItem::separator()))
                .chain(self.palettes.iter().map(|(id, name)| {
                    item(name, Intent::Palette(Some(id.clone())))
                        .checked(self.palette_id.as_ref() == Some(id))
                })),
            ),
        )
    }
    fn menus(&self) -> Vec<Menu> {
        let Self {
            ready,
            connected,
            raw,
            status,
            selected_slot,
            ..
        } = *self;
        let [firmware_recent, eeprom_recent] = self.recents();
        let mut files = vec![
            item("New Window", Intent::NewWindow),
            item("Open Firmware…", Intent::OpenFirmware),
            item("New EEPROM", Intent::NewEeprom),
            item("Open EEPROM…", Intent::OpenEeprom),
            firmware_recent,
            eeprom_recent,
            MenuItem::separator(),
            self.active("Save EEPROM", Intent::SaveEeprom)
                .disabled(!ready || !self.modified || self.saving),
            self.active("Save EEPROM As…", Intent::SaveEepromAs)
                .disabled(!ready || self.saving),
            self.active("Save Screenshot…", Intent::Screenshot),
            MenuItem::separator(),
            item("Close Window", Intent::Close),
        ];
        let mut tools = vec![
            self.active(
                "Walking Simulation…",
                Intent::Tool(crate::tools::Kind::Motion),
            ),
            self.infrared_menu(),
        ];
        if !cfg!(target_os = "macos") {
            tools.push(item("Settings…", Intent::Settings));
            files.push(item("Exit", Intent::Quit));
        }
        let mut menus = vec![
            Menu::new("File").items(files),
            Menu::new("Emulation").items(
                self.playback().into_iter().chain([
                    MenuItem::separator(),
                    MenuItem::submenu(Menu::new("Speed").items(self.speeds())),
                    self.active("Fast-forward", Intent::ToggleTurbo)
                        .checked(self.turbo)
                        .disabled(!ready || connected),
                    MenuItem::separator(),
                    self.load_menu(),
                    self.save_menu(),
                    MenuItem::submenu(Menu::new("Select State Slot").items((1..=10).map(|slot| {
                        self.active(&format!("Slot {slot}"), Intent::SelectSlot(slot))
                            .disabled(!self.has_eeprom)
                            .checked(slot == selected_slot)
                    }))),
                    item("Undo Load State", Intent::UndoLoadState)
                        .disabled(!ready || !self.can_undo),
                    MenuItem::separator(),
                    self.active("Mute Audio", Intent::Mute).checked(self.muted),
                ]),
            ),
            Menu::new("View").items([
                self.scale_menu(),
                self.palette_menu(),
                self.active("Fit Window to Display", Intent::Fit),
                item("Fullscreen", Intent::Fullscreen),
                MenuItem::separator(),
                self.active("Show Status Bar", Intent::Status)
                    .checked(status),
                self.active("Raw Drive", Intent::Raw).checked(raw),
            ]),
            Menu::new("Tools").items(tools),
            Menu::new("Help").items([
                item("Controls…", Intent::Controls),
                item("Documentation", Intent::Documentation),
                MenuItem::separator(),
                item("Report a Problem…", Intent::Report),
            ]),
        ];
        if cfg!(target_os = "macos") {
            menus.insert(
                0,
                Menu::new("hachistep").items([
                    item("About hachistep…", Intent::About),
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
                .push(item("About hachistep…", Intent::About));
        }
        menus
    }

    pub fn context(&self) -> Vec<MenuItem> {
        self.playback()
            .into_iter()
            .chain([
                self.load_menu(),
                self.save_menu(),
                MenuItem::separator(),
                self.scale_menu(),
                self.palette_menu(),
                self.active("Save Screenshot…", Intent::Screenshot),
                MenuItem::separator(),
                self.active(
                    "Walking Simulation…",
                    Intent::Tool(crate::tools::Kind::Motion),
                ),
                self.infrared_menu(),
            ])
            .collect()
    }
}

pub(crate) fn native(
    items: impl IntoIterator<Item = MenuItem>,
) -> gpui_kit::component::native_menu::NativeMenu {
    use gpui_kit::component::native_menu::NativeMenu;
    items
        .into_iter()
        .fold(NativeMenu::new(), |menu, item| match item {
            MenuItem::Separator => menu.separator(),
            MenuItem::Submenu(submenu) => menu.submenu(submenu.name, native(submenu.items)),
            MenuItem::Action {
                name,
                action,
                disabled,
                checked,
                ..
            } => {
                if disabled {
                    menu.menu_with_disabled(name, true, action)
                } else {
                    menu.menu_with_check(name, checked, action)
                }
            }
            MenuItem::SystemMenu(_) => menu,
        })
}
