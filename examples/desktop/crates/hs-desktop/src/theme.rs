use gpui_kit::{
    component::{ActiveTheme, Theme, ThemeMode, TitleBar},
    *,
};

pub(crate) const FONT_SIZE: Pixels = px(14.);

pub(crate) fn status_height(rem: Pixels) -> Pixels {
    rem * 1.5
}

pub(crate) fn menu_height(rem: Pixels) -> Pixels {
    if cfg!(target_os = "macos") {
        px(0.)
    } else {
        rem * 2.
    }
}

pub(crate) fn player_chrome(rem: Pixels, status: bool) -> Pixels {
    gpui_kit::component::TITLE_BAR_HEIGHT
        + menu_height(rem)
        + if status { status_height(rem) } else { px(0.) }
}

pub(crate) fn apply(appearance: usize, cx: &mut App) {
    let mode = match appearance {
        1 => ThemeMode::Light,
        2 => ThemeMode::Dark,
        _ => cx.window_appearance().into(),
    };
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        let color = |light, night| rgb(if dark { night } else { light }).into();
        let c = &mut theme.colors;
        c.background = color(0xfaf9f6, 0x232420);
        c.foreground = color(0x282b25, 0xeeede7);
        c.muted = color(0xebeae4, 0x33352f);
        c.muted_foreground = color(0x686d61, 0xa8ad9f);
        c.border = color(0xdcded5, 0x42463c);
        c.input = c.border;
        c.popover = color(0xfffefa, 0x2c2e28);
        c.popover_foreground = c.foreground;
        c.primary = color(0x9b6400, 0xe9b454);
        c.primary_foreground = color(0xffffff, 0x27281f);
        c.primary_hover = color(0x855600, 0xf2c577);
        c.primary_active = color(0x704900, 0xd8a246);
        c.accent = color(0xf1e7d0, 0x49412c);
        c.accent_foreground = c.foreground;
        c.secondary = c.muted;
        c.secondary_foreground = c.foreground;
        c.secondary_hover = c.border;
        c.button = c.popover;
        c.button_foreground = c.foreground;
        c.button_hover = c.muted;
        c.button_active = c.border;
        c.button_primary = c.primary;
        c.button_primary_foreground = c.primary_foreground;
        c.button_primary_hover = c.primary_hover;
        c.button_primary_active = c.primary_active;
        c.ring = c.primary;
        c.caret = c.foreground;
        c.selection = c.accent;
        c.sidebar = color(0xf0f0e9, 0x292b25);
        c.sidebar_foreground = c.foreground;
        c.sidebar_border = c.border;
        c.sidebar_accent = c.accent;
        c.sidebar_accent_foreground = c.foreground;
        c.sidebar_primary = c.primary;
        c.sidebar_primary_foreground = c.primary_foreground;
        c.title_bar = c.background;
        c.title_bar_border = c.border;
        c.status_bar = c.background;
        c.status_bar_border = c.border;
        c.list = c.background;
        c.list_hover = c.muted;
        c.list_active = c.accent;
        c.list_active_border = c.border;
        c.group_box = c.background;
        c.group_box_foreground = c.foreground;
        c.description_list_label = c.background;
        c.description_list_label_foreground = c.muted_foreground;
        c.slider_bar = c.muted;
        c.slider_thumb = c.primary;
        c.switch = c.muted;
        c.tab_bar = c.background;
        c.tab_bar_segmented = c.muted;
        c.tab_active = c.popover;
        c.tab_active_foreground = c.foreground;
        c.tab_foreground = c.muted_foreground;
        c.chart_1 = c.primary;
        c.chart_2 = color(0x527973, 0x85b3a6);
        c.chart_3 = color(0x777087, 0xb1a2c2);
        theme.font_size = FONT_SIZE;
    });
}

pub(crate) fn title(name: impl Into<SharedString>, window: &Window, cx: &App) -> TitleBar {
    TitleBar::new().bg(cx.theme().background).child(
        div()
            .flex_1()
            .min_w_0()
            .text_ellipsis()
            .pr_4()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(if window.is_window_active() {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            })
            .child(name.into()),
    )
}
