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
    TcpConnect,
    TcpListen,
    Disconnect,
    Motion,
    Settings,
    Diagnostics,
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
        ("s", Intent::SaveSelectedState),
        ("l", Intent::LoadSelectedState),
        ("shift-s", Intent::SaveState),
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
    let active = |name: &str, intent: Intent| item(name, intent).disabled(!ready);
    let mut files = vec![
        item("New Window", Intent::NewWindow),
        item("Open Firmware…", Intent::OpenFirmware),
        item("Open EEPROM…", Intent::OpenEeprom),
        item("New EEPROM…", Intent::NewEeprom),
        MenuItem::submenu(Menu::new("Open Recent").items(recent.iter().map(|pair| {
            item(
                &crate::settings::filename(&pair.eeprom),
                Intent::Recent(pair.clone()),
            )
        }))),
        MenuItem::separator(),
        active("Save EEPROM As…", Intent::SaveEepromAs),
        active("Save Screenshot…", Intent::Screenshot),
        MenuItem::separator(),
        item("Close Window", Intent::Close),
    ];
    let mut tools = vec![
        active("Walking Simulation…", Intent::Motion),
        MenuItem::submenu(Menu::new("Infrared").items([
            active("Connect to Local Window…", Intent::LocalLink),
            active("Connect over TCP…", Intent::TcpConnect),
            active("Listen over TCP…", Intent::TcpListen),
            active("Disconnect", Intent::Disconnect),
        ])),
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
            MenuItem::submenu(
                Menu::new("Speed").items([50, 75, 100, 150, 200, 400, 800].map(|percent| {
                    item(&format!("{percent}%"), Intent::Speed(percent))
                        .checked(percent == frame.speed_percent)
                        .disabled(
                            !ready
                                || (percent != 100
                                    && frame.connection == hs_runtime::Connection::Connected),
                        )
                })),
            ),
            active("Fast-forward", Intent::ToggleTurbo)
                .checked(frame.turbo)
                .disabled(!ready || frame.connection == hs_runtime::Connection::Connected),
            MenuItem::separator(),
            MenuItem::submenu(
                Menu::new("Load State").items(
                    std::iter::once(
                        active("Selected Slot", Intent::LoadSelectedState)
                            .disabled(!ready || !frame.slots[usize::from(selected_slot - 1)]),
                    )
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
                    std::iter::once(active("Selected Slot", Intent::SaveSelectedState))
                        .chain((1..=10).map(|slot| {
                            active(&format!("Slot {slot}"), Intent::SaveSlot(slot))
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
            item("Show Status Bar", Intent::Status).checked(status),
            item("Raw Drive", Intent::Raw).checked(raw),
        ]),
        Menu::new("Tools").items(tools),
        Menu::new("Help").items([
            item("Controls…", Intent::Controls),
            item("Documentation", Intent::Documentation),
            item("Report a Problem…", Intent::Report),
        ]),
    ];
    if cfg!(target_os = "macos") {
        menus.insert(
            0,
            Menu::new("hachistep").items([
                item("About hachistep…", Intent::About),
                item("Settings…", Intent::Settings),
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
