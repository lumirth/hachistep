use gpui_kit::*;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) enum Intent {
    NewWindow,
    FocusWindow(u64),
    OpenFirmware,
    OpenSave,
    NewSave,
    Recent(crate::settings::Recent),
    Recents,
    SaveAs,
    Export,
    SaveState,
    LoadState,
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
        ("shift-o", Intent::OpenSave),
        ("s", Intent::SaveState),
        ("l", Intent::LoadState),
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
    windows: &[crate::OpenWindow],
) -> Vec<Menu> {
    let ready = frame.ready;
    let paused = frame.paused;
    let walking = frame.walking;
    let active = |name: &str, intent: Intent| item(name, intent).disabled(!ready);
    let mut files = vec![
        item("New Window", Intent::NewWindow),
        item("Open Firmware…", Intent::OpenFirmware),
        item("Open Save Data…", Intent::OpenSave),
        item("New Save Data…", Intent::NewSave),
        MenuItem::submenu(Menu::new("Open Recent").items(recent.iter().map(|pair| {
            item(
                &crate::settings::filename(&pair.save),
                Intent::Recent(pair.clone()),
            )
        }))),
        MenuItem::separator(),
        active("Save Data As…", Intent::SaveAs),
        active("Export EEPROM…", Intent::Export),
        active(
            if frame.peer.is_some() {
                "Save Linked State…"
            } else {
                "Save State…"
            },
            Intent::SaveState,
        ),
        active("Load State…", Intent::LoadState),
        active("Save Screenshot…", Intent::Screenshot),
        MenuItem::separator(),
        item("Close Window", Intent::Close),
    ];
    let mut tools = vec![
        active("Motion Controls…", Intent::Motion),
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
            active(
                if walking {
                    "Stop Walking"
                } else {
                    "Start Walking"
                },
                Intent::Walking,
            ),
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
        Menu::new("Link").items([
            active("Connect to Local Window…", Intent::LocalLink),
            active("Connect over TCP…", Intent::TcpConnect),
            active("Listen over TCP…", Intent::TcpListen),
            active("Disconnect", Intent::Disconnect),
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
            Menu::new("Window").items(
                std::iter::once(item("New Window", Intent::NewWindow)).chain(windows.iter().map(
                    |window| {
                        item(
                            &window.name,
                            Intent::FocusWindow(window.view.entity_id().as_u64()),
                        )
                    },
                )),
            ),
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
