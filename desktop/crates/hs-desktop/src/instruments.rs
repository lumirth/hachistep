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

pub(crate) fn infrared(frame: &Presentation, cx: &App) -> impl IntoElement {
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
            for (lane, color) in colors.into_iter().enumerate() {
                let baseline = height * (0.3 + lane as f32 * 0.5);
                window.paint_quad(fill(
                    Bounds::new(
                        bounds.origin + point(px(0.), px(baseline)),
                        size(bounds.size.width, px(pixel)),
                    ),
                    grid,
                ));
                for &(at, transmit) in activity.iter() {
                    let age = now.saturating_sub(at.raw()) as f64 / 18446744073709551616.;
                    if usize::from(!transmit) != lane || age > 1. {
                        continue;
                    }
                    let x = width * (1. - age as f32);
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin
                                + point(px(x.min(width - pixel)), px(baseline - height * 0.2)),
                            size(px(pixel), px(height * 0.2)),
                        ),
                        color,
                    ));
                }
            }
        },
    )
    .size_full()
}
