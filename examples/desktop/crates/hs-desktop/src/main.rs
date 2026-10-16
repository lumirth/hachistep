extern crate gpui_kit as gpui;
mod commands;
mod lcd;
mod player;
mod preferences;
mod settings;
mod tools;

use gpui_kit::*;
use hs_runtime::Runtime;
use player::PlayerView;
use settings::Preferences;
use std::{path::PathBuf, sync::mpsc::Sender};

struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match path {
            "icon.png" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/icon.png"
            )))),
            _ => gpui_kit::assets::Assets.load(path),
        }
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
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
    quitting: bool,
    preferences_window: Option<AnyWindowHandle>,
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
            appears_transparent: false,
            ..Default::default()
        }),
        window_min_size: Some(size(px(288.), px(192.))),
        app_id: Some("me.kukas.hachistep".into()),
        ..Default::default()
    }
}

enum Launch {
    Firmware(PathBuf),
    Files {
        pair: settings::Recent,
        create: bool,
    },
}

fn new_window(cx: &mut App, launch: Option<Launch>) {
    cx.defer(move |cx| create_window(cx, launch));
}

fn create_window(cx: &mut App, launch: Option<Launch>) {
    let (firmware, files) = match launch {
        Some(Launch::Firmware(firmware)) => (Some(firmware), None),
        Some(Launch::Files { pair, create }) => (None, Some((pair, create))),
        None => (None, None),
    };
    let runtime = cx.global::<Desktop>().runtime.clone();
    match gpui_kit::open_window(options("hachistep", 384., 256.), cx, |window, cx| {
        cx.new(|cx| PlayerView::new(runtime.player(), firmware, window, cx))
    }) {
        Ok((handle, view)) => {
            cx.global_mut::<Desktop>().windows.push(OpenWindow {
                handle,
                view: view.downgrade(),
            });
            if let Some((pair, create)) = files {
                view.update(cx, |view, cx| view.open_pair(pair, create, cx));
            }
        }
        Err(error) => eprintln!("Could not open player: {error:#}"),
    }
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
    let views: Vec<_> = cx
        .global::<Desktop>()
        .windows
        .iter()
        .filter_map(|w| w.view.upgrade())
        .collect();
    if views.is_empty() {
        cx.quit();
    }
    for view in views {
        view.update(cx, |view, _| view.begin_close());
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
        (Some(firmware), Some(eeprom)) => Some(settings::Recent { firmware, eeprom }),
        (None, None) => None,
        _ => {
            eprintln!("Use --firmware and --eeprom together.");
            std::process::exit(2);
        }
    };
    let platform = gpui_kit::platform::current_platform(false);
    Application::with_platform(platform.clone())
        .with_assets(Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("me.kukas.hachistep", "hachistep");
            let (preferences, persist) = Preferences::load();
            let mode = match preferences.appearance {
                1 => component::ThemeMode::Light,
                2 => component::ThemeMode::Dark,
                _ => cx.window_appearance().into(),
            };
            component::Theme::change(mode, None, cx);
            cx.set_global(Desktop {
                runtime: Runtime::new(),
                preferences,
                persist,
                windows: Vec::new(),
                quitting: false,
                preferences_window: None,
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
            commands::bind(cx);
            cx.set_menus(commands::menus(
                &hs_runtime::Presentation::default(),
                false,
                false,
                4,
                &cx.global::<Desktop>().preferences.recent,
                1,
            ));
            cx.on_action(|action: &commands::Invoke, cx| match action.intent {
                commands::Intent::NewWindow => new_window(cx, None),
                commands::Intent::Quit => quit(cx),
                commands::Intent::Settings => preferences::open(cx),
                _ => (),
            });
            create_window(
                cx,
                pair.clone().map(|pair| Launch::Files {
                    pair,
                    create: false,
                }),
            );
            cx.activate(true);
        });
}
