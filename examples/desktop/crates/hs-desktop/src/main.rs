extern crate gpui_kit as gpui;
mod commands;
mod instruments;
mod lcd;
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
    active_player: Option<WeakEntity<PlayerView>>,
    quitting: bool,
    disabled_commands: Vec<commands::Intent>,
    menu: Option<commands::MenuState>,
    preferences_window: Option<(AnyWindowHandle, WeakEntity<preferences::Settings>)>,
    tools: BTreeMap<(hs_runtime::Id, tools::Kind), AnyWindowHandle>,
    tool_bounds: BTreeMap<(hs_runtime::Id, tools::Kind), (Bounds<Pixels>, bool)>,
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
        None => (None, None, None),
    };
    let runtime = cx.global::<Desktop>().runtime.clone();
    let mut window_options = options(
        "hachistep",
        384.,
        256. + f32::from(theme::player_chrome(theme::FONT_SIZE, false)),
    );
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
    } else {
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
                &cx.global::<Desktop>().preferences.recent,
                1,
            )
            .install(cx);
            cx.on_action(|action: &commands::Intent, cx| match *action {
                commands::Intent::NewWindow => new_window(cx, None),
                commands::Intent::Quit => quit(cx),
                commands::Intent::Settings => preferences::open(cx),
                _ => (),
            });
            create_window(cx, pair.clone().map(Launch::Files));
            cx.activate(true);
        });
}
