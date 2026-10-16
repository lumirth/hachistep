#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
extern crate gpui_kit as gpui;
mod about;
mod commands;
mod firmware;
mod instruments;
mod lcd;
mod macos;
mod palettes;
mod player;
mod preferences;
mod settings;
mod theme;
mod tools;

use gpui_kit::*;
use hs_runtime::Runtime;
use player::PlayerView;
use settings::Preferences;
use std::{collections::BTreeMap, path::PathBuf, sync::mpsc::Sender};

gpui_kit::assets::icon_assets!(
    DesktopIcons,
    [Clock3, Save, Footprints, Radio, Network, Gauge]
);

struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match path {
            "wordmark.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../../assets/logo-two-tone.svg"
            )))),
            "icon.png" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../../assets/icons/macos/icon.png"
            )))),
            _ => match DesktopIcons.load(path)? {
                Some(icon) => Ok(Some(icon)),
                None => gpui_kit::assets::Assets.load(path),
            },
        }
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(DesktopIcons.list(path)?);
        Ok(paths)
    }
}

struct OpenWindow {
    handle: AnyWindowHandle,
    view: WeakEntity<PlayerView>,
}
struct Desktop {
    runtime: Runtime,
    preferences: Preferences,
    persist: Sender<Preferences>,
    windows: Vec<OpenWindow>,
    active_player: Option<WeakEntity<PlayerView>>,
    quitting: bool,
    disabled_commands: Vec<commands::Intent>,
    menu: Option<commands::MenuState>,
    preferences_window: Option<(AnyWindowHandle, WeakEntity<preferences::Settings>)>,
    tools: BTreeMap<(hs_runtime::Id, tools::Kind), AnyWindowHandle>,
    tool_bounds: BTreeMap<(hs_runtime::Id, tools::Kind), Bounds<Pixels>>,
    about_window: Option<AnyWindowHandle>,
    tcp_options: BTreeMap<hs_runtime::Id, tools::TcpOptions>,
}
impl Global for Desktop {}

fn options(title: &str, width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(200.), px(180.)),
            size(px(width), px(height)),
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            ..component::TitleBar::title_bar_options()
        }),
        window_min_size: Some(size(px(288.), px(192.))),
        app_id: Some("me.kukas.hachistep".into()),
        ..component::TitleBar::window_options()
    }
}

enum Launch {
    Firmware(PathBuf),
    Files(settings::Recent),
    Blank(PathBuf),
}

fn new_window(cx: &mut App, launch: Option<Launch>) {
    cx.defer(move |cx| create_window(cx, launch));
}

fn create_window(cx: &mut App, launch: Option<Launch>) {
    let (firmware, files, blank) = match launch {
        Some(Launch::Firmware(firmware)) => (Some(firmware), None, None),
        Some(Launch::Files(pair)) => (None, Some(pair), None),
        Some(Launch::Blank(firmware)) => (Some(firmware.clone()), None, Some(firmware)),
        None => (
            cx.global::<Desktop>()
                .preferences
                .default_firmware
                .as_ref()
                .and_then(firmware::Firmware::path),
            None,
            None,
        ),
    };
    let runtime = cx.global::<Desktop>().runtime.clone();
    let mut window_options = options(
        "hachistep",
        384. + f32::from(cx.global::<Desktop>().preferences.lcd_margin) * 2.,
        256. + f32::from(cx.global::<Desktop>().preferences.lcd_margin) * 2.
            + f32::from(theme::player_chrome(theme::FONT_SIZE, false)),
    );
    if cfg!(target_os = "macos") {
        window_options.window_min_size = Some(size(
            px(96. + f32::from(cx.global::<Desktop>().preferences.lcd_margin) * 2.),
            px(64. + f32::from(cx.global::<Desktop>().preferences.lcd_margin) * 2.)
                + theme::player_chrome(theme::FONT_SIZE, false),
        ));
    }
    if cx.global::<Desktop>().windows.is_empty() {
        if let Some([x, y, width, height]) = cx
            .global::<Desktop>()
            .preferences
            .player_bounds
            .filter(|values| values.iter().all(|v| v.is_finite()))
        {
            let origin = point(px(x), px(y));
            if let Some(display) = cx
                .displays()
                .into_iter()
                .find(|display| display.bounds().contains(&origin))
                .or_else(|| cx.primary_display())
            {
                let screen = display.bounds();
                let size = size(
                    px(width).max(px(192.)).min(screen.size.width),
                    px(height).max(px(160.)).min(screen.size.height),
                );
                let origin = point(
                    origin.x.max(screen.left()).min(screen.right() - size.width),
                    origin
                        .y
                        .max(screen.top())
                        .min(screen.bottom() - size.height),
                );
                window_options.window_bounds =
                    Some(WindowBounds::Windowed(Bounds::new(origin, size)));
                window_options.display_id = Some(display.id());
            }
        }
    }
    let previous = cx
        .global::<Desktop>()
        .active_player
        .as_ref()
        .and_then(|active| {
            cx.global::<Desktop>()
                .windows
                .iter()
                .find(|window| window.view.entity_id() == active.entity_id())
                .map(|window| window.handle)
        });
    if let Some(Ok((bounds, screen))) = previous.map(|handle| {
        handle.update(cx, |_, window, cx| {
            (
                window.bounds(),
                window.display(cx).map(|display| display.bounds()),
            )
        })
    }) {
        if let Some(WindowBounds::Windowed(next)) = &mut window_options.window_bounds {
            next.origin = bounds.origin + point(px(24.), px(24.));
            if let Some(screen) = screen {
                next.origin.x = next
                    .origin
                    .x
                    .min(screen.right() - next.size.width)
                    .max(screen.origin.x);
                next.origin.y = next
                    .origin
                    .y
                    .min(screen.bottom() - next.size.height)
                    .max(screen.origin.y);
            }
        }
    }
    match gpui_kit::open_window(window_options, cx, |window, cx| {
        cx.new(|cx| PlayerView::new(runtime.player(), firmware, window, cx))
    }) {
        Ok((handle, view)) => {
            cx.global_mut::<Desktop>().windows.push(OpenWindow {
                handle,
                view: view.downgrade(),
            });
            if let Some(pair) = files {
                view.update(cx, |view, cx| view.open_pair(pair, cx));
            } else if let Some(firmware) = blank {
                view.update(cx, |view, cx| view.open_image(firmware, None, cx));
            }
        }
        Err(error) => eprintln!("Could not open player: {error:#}"),
    }
}

fn palette_for(id: hs_runtime::Id, cx: &App) -> lcd::Palette {
    cx.global::<Desktop>()
        .windows
        .iter()
        .filter_map(|window| window.view.upgrade())
        .find_map(|view| {
            let view = view.read(cx);
            (view.player().id() == id).then(|| view.palette(cx))
        })
        .unwrap_or(cx.global::<Desktop>().preferences.palette)
}

fn system_action(intent: &commands::Intent, window: &mut Window, cx: &mut App) -> bool {
    use commands::Intent;
    match intent {
        Intent::Hide => cx.hide(),
        Intent::HideOthers => cx.hide_other_apps(),
        Intent::ShowAll => cx.unhide_other_apps(),
        Intent::Minimize => window.minimize_window(),
        Intent::Zoom => window.zoom_window(),
        Intent::BringAll => macos::bring_all(),
        _ => return false,
    }
    true
}

fn opening_window(cx: &mut App) -> Option<(AnyWindowHandle, WeakEntity<PlayerView>)> {
    cx.global::<Desktop>()
        .windows
        .iter()
        .find(|window| {
            window
                .view
                .upgrade()
                .is_some_and(|view| view.read(cx).accepts_file())
        })
        .map(|w| (w.handle, w.view.clone()))
}
fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    for path in paths {
        let canonical = path.canonicalize().ok();
        let existing = cx
            .global::<Desktop>()
            .windows
            .iter()
            .find(|window| {
                window.view.upgrade().is_some_and(|view| {
                    canonical.is_some()
                        && view.read(cx).presentation().eeprom.as_ref() == canonical.as_ref()
                })
            })
            .map(|w| w.handle);
        if let Some(handle) = existing {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            continue;
        }
        let (handle, view) = opening_window(cx).unwrap_or_else(|| {
            create_window(cx, None);
            let window = cx.global::<Desktop>().windows.last().unwrap();
            (window.handle, window.view.clone())
        });
        let _ = handle.update(cx, |_, window, cx| {
            let _ = view.update(cx, |view, cx| {
                if std::fs::metadata(&path).is_ok_and(|meta| meta.len() == 65_536) {
                    let known = cx
                        .global::<Desktop>()
                        .preferences
                        .recent
                        .iter()
                        .find(|recent| recent.eeprom == path)
                        .cloned();
                    view.open_pair(
                        known.unwrap_or_else(|| settings::Recent {
                            firmware: cx
                                .global::<Desktop>()
                                .preferences
                                .default_firmware
                                .as_ref()
                                .and_then(firmware::Firmware::path)
                                .unwrap_or_else(|| "firmware.bin".into()),
                            firmware_hash: cx
                                .global::<Desktop>()
                                .preferences
                                .default_firmware
                                .as_ref()
                                .map(|f| f.hash.clone()),
                            eeprom: path,
                        }),
                        cx,
                    );
                } else {
                    view.selected(player::FileCommand::Firmware, path, window, cx);
                }
            });
            window.activate_window();
        });
    }
}

pub(crate) fn application_action(intent: &commands::Intent, cx: &mut App) {
    use commands::Intent;
    match intent {
        Intent::NewWindow => new_window(cx, None),
        Intent::Quit => quit(cx),
        Intent::Settings => preferences::open(cx),
        Intent::About => about::open(cx),
        Intent::Hide => cx.hide(),
        Intent::HideOthers => cx.hide_other_apps(),
        Intent::ShowAll => cx.unhide_other_apps(),
        Intent::BringAll => macos::bring_all(),
        Intent::OpenFirmware
        | Intent::OpenEeprom
        | Intent::NewEeprom
        | Intent::RecentFirmware(_)
        | Intent::RecentEeprom(_) => {
            create_window(cx, None);
            let window = cx
                .global::<Desktop>()
                .windows
                .last()
                .map(|w| (w.handle, w.view.clone()));
            if let Some((handle, view)) = window {
                let intent = intent.clone();
                let _ = handle.update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| view.execute(intent, window, cx))
                });
            }
        }
        _ => (),
    }
}
fn install_empty_menu(cx: &mut App) {
    commands::MenuState::new(
        &hs_runtime::Presentation::default(),
        false,
        false,
        4,
        &cx.global::<Desktop>().preferences,
        1,
    )
    .install(cx);
}

fn invoke_player(owner: &WeakEntity<PlayerView>, intent: commands::Intent, cx: &mut App) {
    let handle = cx
        .global::<Desktop>()
        .windows
        .iter()
        .find(|w| w.view.entity_id() == owner.entity_id())
        .map(|w| w.handle);
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, cx| {
            owner.update(cx, |owner, cx| owner.execute(intent, window, cx))
        });
    }
}

fn player_menu(owner: &WeakEntity<PlayerView>, cx: &mut App) {
    let _ = owner.update(cx, |owner, cx| owner.install_menu(cx));
}

fn save_preferences(cx: &mut App) {
    let desktop = cx.global::<Desktop>();
    let _ = desktop.persist.send(desktop.preferences.clone());
}

fn quit(cx: &mut App) {
    cx.defer(close_players);
}

fn close_players(cx: &mut App) {
    cx.global_mut::<Desktop>().quitting = true;
    let next = cx
        .global::<Desktop>()
        .windows
        .first()
        .map(|window| (window.handle, window.view.clone()));
    if let Some((handle, view)) = next {
        let _ = handle.update(cx, |_, window, cx| {
            let _ = view.update(cx, |view, cx| view.request_close(window, cx));
        });
    } else if !macos::finish_quit(true) {
        cx.quit();
    }
}

fn main() {
    let mut firmware = None;
    let mut eeprom = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--firmware") => firmware = args.next().map(PathBuf::from),
            Some("--eeprom") => eeprom = args.next().map(PathBuf::from),
            Some("--help") => {
                println!("hachistep [--firmware FILE --eeprom FILE]");
                return;
            }
            _ => {
                eprintln!("Unknown argument: {}", argument.to_string_lossy());
                std::process::exit(2);
            }
        }
    }
    let pair = match (firmware, eeprom) {
        (Some(firmware), Some(eeprom)) => Some(settings::Recent {
            firmware_hash: firmware::identify(&firmware).ok(),
            firmware,
            eeprom,
        }),
        (None, None) => None,
        _ => {
            eprintln!("Use --firmware and --eeprom together.");
            std::process::exit(2);
        }
    };
    let platform = gpui_kit::platform::current_platform(false);
    let initial_files = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let open_context = std::rc::Rc::new(std::cell::RefCell::new(None::<AsyncApp>));
    let application = Application::with_platform(platform.clone()).with_assets(Assets);
    let pending_files = initial_files.clone();
    let file_context = open_context.clone();
    application.on_open_urls(move |urls| {
        let paths: Vec<_> = urls
            .into_iter()
            .filter_map(|url| url::Url::parse(&url).ok()?.to_file_path().ok())
            .collect();
        if let Some(context) = file_context.borrow().as_ref() {
            context.update(|cx| cx.defer(move |cx| open_paths(paths, cx)));
        } else {
            pending_files.borrow_mut().extend(paths);
        }
    });
    application.on_reopen(|cx| {
        if macos::restore_minimized() {
            return;
        }
        if cx.global::<Desktop>().windows.is_empty() {
            new_window(cx, None);
        }
    });
    application.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_app_identity("me.kukas.hachistep", "hachistep");
        let (preferences, persist) = Preferences::load();
        theme::apply(preferences.appearance, cx);
        let runtime = Runtime::new();
        runtime.sound_enabled(!preferences.mute);
        cx.set_global(Desktop {
            runtime,
            preferences,
            persist,
            windows: Vec::new(),
            active_player: None,
            quitting: false,
            disabled_commands: Vec::new(),
            menu: None,
            preferences_window: None,
            about_window: None,
            tools: BTreeMap::new(),
            tool_bounds: BTreeMap::new(),
            tcp_options: BTreeMap::new(),
        });
        let menu_context = cx.to_async();
        platform.on_will_open_app_menu(Box::new(move || {
            menu_context.update(|cx| {
                let views: Vec<_> = cx
                    .global::<Desktop>()
                    .windows
                    .iter()
                    .filter_map(|w| w.view.upgrade())
                    .collect();
                for view in views {
                    view.update(cx, |view, _| view.release());
                }
            });
        }));
        let validation_context = cx.to_async();
        platform.on_validate_app_menu_command(Box::new(move |action| {
            validation_context.update(|cx| {
                if let Some(action) = action.as_any().downcast_ref::<commands::Intent>() {
                    !cx.global::<Desktop>().disabled_commands.contains(action)
                } else {
                    cx.is_action_available(action)
                }
            })
        }));
        commands::bind(cx);
        commands::MenuState::new(
            &hs_runtime::Presentation::default(),
            false,
            false,
            4,
            &cx.global::<Desktop>().preferences,
            1,
        )
        .install(cx);
        cx.on_action(application_action);
        macos::install(cx);
        *open_context.borrow_mut() = Some(cx.to_async());
        let paths = initial_files.take();
        if paths.is_empty() {
            create_window(cx, pair.clone().map(Launch::Files));
        } else {
            open_paths(paths, cx);
        }
        cx.activate(true);
    });
}
