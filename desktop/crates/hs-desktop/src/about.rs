use crate::{commands::Intent, theme, Desktop};
use gpui_kit::{
    base::Link,
    component::{v_flex, ActiveTheme},
    *,
};

pub(crate) fn open(cx: &mut App) {
    cx.defer(open_window);
}

fn open_window(cx: &mut App) {
    if cx.global::<Desktop>().about_window.is_some_and(|handle| {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    }) {
        return;
    }
    let mut options = crate::options("About hachistep", 320., 340.);
    options.is_resizable = false;
    options.is_minimizable = false;
    options.window_min_size = None;
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle().tab_stop(true);
            focus.focus(window, cx);
            let release = cx.on_release(|_, cx| cx.global_mut::<Desktop>().about_window = None);
            About {
                focus,
                _release: release,
            }
        })
    }) {
        Ok((handle, _)) => cx.global_mut::<Desktop>().about_window = Some(handle),
        Err(error) => eprintln!("Could not open About: {error:#}"),
    }
}

struct About {
    focus: FocusHandle,
    _release: Subscription,
}

impl Render for About {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .id("about")
            .tab_group()
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, intent: &Intent, window, cx| match intent {
                Intent::Close => window.remove_window(),
                Intent::Quit => crate::quit(cx),
                Intent::Settings => crate::preferences::open(cx),
                Intent::NewWindow => crate::new_window(cx, None),
                _ => {
                    if let Some(owner) = cx.global::<Desktop>().active_player.clone() {
                        crate::invoke_player(&owner, intent.clone(), cx);
                    }
                }
            }))
            .on_key_down(|event, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            })
            .child(theme::title("About hachistep", window, cx))
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .p_6()
                    .child(img("icon.png").size_20())
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("hachistep"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                    )
                    .child("Pokéwalker emulator")
                    .child(
                        Link::new("project")
                            .accessibility_label("GitHub")
                            .child("GitHub")
                            .href("https://github.com/lumirth/hachistep")
                            .open_with(|url, _, _, cx| cx.open_url(url))
                            .text_color(cx.theme().link)
                            .text_decoration_1()
                            .cursor_pointer()
                            .rounded_sm()
                            .px_1()
                            .focus_visible(|style| style.bg(cx.theme().accent))
                            .hover(|style| style.text_color(cx.theme().link.opacity(0.8))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("MIT license"),
                    ),
            )
    }
}
