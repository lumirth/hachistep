use gpui_kit::*;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) enum Intent {
    NewWindow,
    OpenFirmware,
    OpenEeprom,
    NewEeprom,
    Recent(crate::settings::Recent),
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
    Walking,
    Speed(u16),
    Mute,
    Scale(u8),
    Fit,
    Fullscreen,
    Status,
    Raw,
    LocalLink,
    Connect(u64),
    TcpConnection,
    Disconnect,
    Motion,
    Settings,
    Diagnostics,
    DiagnosticsExport,
    Controls,
    Documentation,
    Report,
    About,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Action)]
#[action(namespace = hachistep, no_json)]
pub(crate) struct Invoke {
    pub(crate) intent: Intent,
}

impl Invoke {
    pub fn new(intent: Intent) -> Self {
        Self { intent }
    }
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
        cx.bind_keys([KeyBinding::new(
            &format!("{modifier}-{key}"),
            Invoke::new(intent),
            None,
        )]);
    }
    for slot in 1..=10 {
        cx.bind_keys([
            KeyBinding::new(
                &format!("f{slot}"),
                Invoke::new(Intent::LoadSlot(slot)),
                None,
            ),
            KeyBinding::new(
                &format!("shift-f{slot}"),
                Invoke::new(Intent::SaveSlot(slot)),
                None,
            ),
        ]);
    }
    cx.bind_keys([KeyBinding::new(
        "f12",
        Invoke::new(Intent::UndoLoadState),
        None,
    )]);
}

fn item(name: &str, intent: Intent) -> MenuItem {
    MenuItem::action(name.to_owned(), Invoke::new(intent))
}

pub(crate) fn install(menus: Vec<Menu>, cx: &mut App) {
    fn collect(items: &[MenuItem], inherited: bool, disabled: &mut Vec<Intent>) {
        for item in items {
            match item {
                MenuItem::Action {
                    action,
                    disabled: item_disabled,
                    ..
                } if inherited || *item_disabled => {
                    if let Some(action) = action.as_any().downcast_ref::<Invoke>() {
                        disabled.push(action.intent.clone());
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
    cx.set_menus(menus);
}

pub(crate) fn menus(
    frame: &hs_runtime::Presentation,
    raw: bool,
    status: bool,
    scale: u8,
    recent: &[crate::settings::Recent],
    selected_slot: u8,
) -> Vec<Menu> {
    let ready = frame.ready;
    let paused = frame.paused;
    let connected = frame.is_connection_active();
    let active = |name: &str, intent: Intent| item(name, intent).disabled(!ready);
    let mut files = vec![
        item("New Window", Intent::NewWindow),
        item("Open Firmware…", Intent::OpenFirmware),
        item("Open EEPROM…", Intent::OpenEeprom),
        MenuItem::submenu(Menu::new("Open Recent").items(recent.iter().map(|pair| {
            item(
                &crate::settings::filename(&pair.eeprom),
                Intent::Recent(pair.clone()),
            )
        }))),
        MenuItem::separator(),
        active("Save EEPROM", Intent::SaveEeprom)
            .disabled(!ready || !frame.modified || frame.saving),
        active("Save EEPROM As…", Intent::SaveEepromAs).disabled(!ready || frame.saving),
        active("Save Screenshot…", Intent::Screenshot),
        MenuItem::separator(),
        item("Close Window", Intent::Close),
    ];
    let mut tools = vec![
        active("Walking Simulation…", Intent::Motion),
        MenuItem::submenu(Menu::new("Infrared").items([
            active("Local Link…", Intent::LocalLink),
            active("TCP Connection…", Intent::TcpConnection),
        ])),
        MenuItem::separator(),
        active("Diagnostics…", Intent::Diagnostics),
    ];
    if !cfg!(target_os = "macos") {
        tools.push(item("Settings…", Intent::Settings));
        files.push(item("Exit", Intent::Quit));
    }
    let mut menus = vec![
        Menu::new("File").items(files),
        Menu::new("Emulation").items([
            active(if paused { "Resume" } else { "Pause" }, Intent::Pause),
            active("Reset", Intent::Reset),
            MenuItem::separator(),
            MenuItem::submenu(
                Menu::new("Speed").items([50, 75, 100, 150, 200, 400, 800].map(|percent| {
                    item(&format!("{percent}%"), Intent::Speed(percent))
                        .checked(percent == frame.speed_percent)
                        .disabled(!ready || (percent != 100 && connected))
                })),
            ),
            active("Fast-forward", Intent::ToggleTurbo)
                .checked(frame.turbo)
                .disabled(!ready || connected),
            MenuItem::separator(),
            MenuItem::submenu(
                Menu::new("Load State").items(
                    std::iter::once(
                        active("Load from Selected Slot", Intent::LoadSelectedState)
                            .disabled(!ready || !frame.slots[usize::from(selected_slot - 1)]),
                    )
                    .chain([MenuItem::separator()])
                    .chain((1..=10).map(|slot| {
                        item(&format!("Slot {slot}"), Intent::LoadSlot(slot))
                            .checked(slot == selected_slot)
                            .disabled(!ready || !frame.slots[usize::from(slot - 1)])
                    }))
                    .chain([
                        MenuItem::separator(),
                        active("Load from File…", Intent::LoadState),
                    ]),
                ),
            ),
            MenuItem::submenu(
                Menu::new("Save State").items(
                    std::iter::once(
                        active("Save to Selected Slot", Intent::SaveSelectedState)
                            .disabled(frame.eeprom.is_none()),
                    )
                    .chain([MenuItem::separator()])
                    .chain((1..=10).map(|slot| {
                        active(&format!("Slot {slot}"), Intent::SaveSlot(slot))
                            .disabled(frame.eeprom.is_none())
                            .checked(slot == selected_slot)
                    }))
                    .chain([
                        MenuItem::separator(),
                        active("Save to File…", Intent::SaveState),
                    ]),
                ),
            ),
            MenuItem::submenu(Menu::new("Select State Slot").items((1..=10).map(|slot| {
                active(&format!("Slot {slot}"), Intent::SelectSlot(slot))
                    .disabled(frame.eeprom.is_none())
                    .checked(slot == selected_slot)
            }))),
            item("Undo Load State", Intent::UndoLoadState).disabled(!ready || !frame.can_undo),
            MenuItem::separator(),
            active("Mute Audio", Intent::Mute).checked(frame.muted),
        ]),
        Menu::new("View").items([
            MenuItem::submenu(Menu::new("Display Scale").items([4, 5, 6, 8].map(|value| {
                item(&format!("{value}×"), Intent::Scale(value)).checked(value == scale)
            }))),
            item("Fit Window to Display", Intent::Fit),
            item("Fullscreen", Intent::Fullscreen),
            MenuItem::separator(),
            item("Show Status Bar", Intent::Status).checked(status),
            item("Raw Drive", Intent::Raw).checked(raw),
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
                item("Quit hachistep", Intent::Quit),
            ]),
        );
        menus.insert(
            menus.len() - 1,
            Menu::new("Window").items([item("New Window", Intent::NewWindow)]),
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

pub(crate) fn context(
    frame: &hs_runtime::Presentation,
    raw: bool,
    status: bool,
    scale: u8,
    selected_slot: u8,
) -> Vec<MenuItem> {
    let mut items = Vec::new();
    for menu in menus(frame, raw, status, scale, &[], selected_slot) {
        match menu.name.as_ref() {
            "Emulation" => {
                items.extend(menu.items.into_iter().filter(|item| match item {
                    MenuItem::Action { name, .. } => {
                        matches!(name.as_ref(), "Pause" | "Resume" | "Reset")
                    }
                    MenuItem::Submenu(menu) => {
                        matches!(menu.name.as_ref(), "Save State" | "Load State")
                    }
                    _ => false,
                }));
                items.push(MenuItem::separator());
            }
            "View" => {
                items.extend(
                    menu.items
                        .into_iter()
                        .filter(|item| matches!(item, MenuItem::Submenu(_))),
                );
                items.push(item("Save Screenshot…", Intent::Screenshot).disabled(!frame.ready));
                items.push(MenuItem::separator());
            }
            "Tools" => items.extend(menu.items.into_iter().filter(|item| match item {
                MenuItem::Action { name, .. } => name.as_ref() == "Walking Simulation…",
                MenuItem::Submenu(menu) => menu.name.as_ref() == "Infrared",
                _ => false,
            })),
            _ => (),
        }
    }
    items
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
