use gpui_kit::{component::ActiveTheme, *};
use hs_runtime::Presentation;

pub(crate) fn motion(frame: &Presentation, cx: &App) -> impl IntoElement {
    let samples = frame.motion_samples.clone();
    let colors = [cx.theme().chart_1, cx.theme().chart_2, cx.theme().chart_3];
    let grid = cx.theme().border;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let stroke = px(1. / window.scale_factor());
            for fraction in [0.25, 0.5, 0.75] {
                window.paint_quad(fill(
                    Bounds::new(
                        bounds.origin + point(px(0.), px(height * fraction)),
                        size(bounds.size.width, stroke),
                    ),
                    grid,
                ));
            }
            if samples.len() < 2 {
                return;
            }
            for (axis, color) in colors.into_iter().enumerate() {
                let mut path = PathBuilder::stroke(stroke);
                for (i, sample) in samples.iter().enumerate() {
                    let value =
                        [sample.x, sample.y, sample.z - 1_000_000][axis] as f32 / 1_200_000.;
                    let at = bounds.origin
                        + point(
                            px(width * i as f32 / 511.),
                            px(height * (0.5 - value.clamp(-1., 1.) * 0.45)),
                        );
                    if i == 0 {
                        path.move_to(at);
                    } else {
                        path.line_to(at);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .size_full()
}

pub(crate) fn infrared_link(
    frame: &Presentation,
    connected: bool,
    failed: bool,
    cx: &App,
) -> impl IntoElement {
    let activity = frame.activity.clone();
    let now = frame.time.raw();
    let colors = [cx.theme().chart_1, cx.theme().chart_2];
    let grid = cx.theme().border;
    let danger = cx.theme().danger;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let pixel = 1. / window.scale_factor();
            for (lane, color) in colors.into_iter().enumerate() {
                let baseline = height * (0.3 + lane as f32 * 0.4);
                let segments = if connected {
                    1
                } else {
                    (width / 8.).ceil() as usize
                };
                for segment in 0..segments {
                    let start = if connected { 0. } else { segment as f32 * 8. };
                    let length = if connected {
                        width
                    } else {
                        4_f32.min(width - start)
                    };
                    if failed && start < width * 0.55 && start + length > width * 0.45 {
                        continue;
                    }
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin + point(px(start), px(baseline)),
                            size(px(length), px(pixel)),
                        ),
                        grid,
                    ));
                }
                if !connected {
                    continue;
                }
                for sample in activity.iter() {
                    let count = if lane == 0 {
                        sample.sent
                    } else {
                        sample.received
                    };
                    let age = now.saturating_sub(sample.at.raw()) as f64 / 18446744073709551616.;
                    if count == 0 || age > 0.7 {
                        continue;
                    }
                    let progress = age as f32 / 0.7;
                    let length = (4. + (count as f32).log2() * 2.).min(width * 0.15);
                    let x = (width - length) * if lane == 0 { progress } else { 1. - progress };
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin + point(px(x), px(baseline - 2.)),
                            size(px(length), px(4.)),
                        ),
                        color,
                    ));
                }
            }
            if failed {
                for direction in [-1., 1.] {
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(
                        bounds.origin
                            + point(px(width * 0.5 - 3.), px(height * 0.5 - direction * 3.)),
                    );
                    path.line_to(
                        bounds.origin
                            + point(px(width * 0.5 + 3.), px(height * 0.5 + direction * 3.)),
                    );
                    if let Ok(path) = path.build() {
                        window.paint_path(path, danger);
                    }
                }
            }
        },
    )
    .size_full()
}

pub(crate) fn infrared_history(frame: &Presentation, cx: &App) -> impl IntoElement {
    let activity = frame.activity.clone();
    let now = frame.time.raw();
    let colors = [cx.theme().chart_1, cx.theme().chart_2];
    let grid = cx.theme().border;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let pixel = 1. / window.scale_factor();
            let mid = height / 2.;
            window.paint_quad(fill(
                Bounds::new(
                    bounds.origin + point(px(0.), px(mid)),
                    size(bounds.size.width, px(pixel)),
                ),
                grid,
            ));
            for sample in activity.iter() {
                let age = now.saturating_sub(sample.at.raw()) as f64 / 18446744073709551616.;
                if age > 12. {
                    continue;
                }
                let x = width * (1. - age as f32 / 12.);
                for (lane, count) in [sample.sent, sample.received].into_iter().enumerate() {
                    if count == 0 {
                        continue;
                    }
                    let extent = (2. + (count as f32).log2()).min(mid - 1.);
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin
                                + point(
                                    px(x.min(width - pixel)),
                                    px(if lane == 0 { mid - extent } else { mid }),
                                ),
                            size(px((width / 384.).max(pixel)), px(extent)),
                        ),
                        colors[lane],
                    ));
                }
            }
        },
    )
    .size_full()
}
