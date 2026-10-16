use gpui_kit::*;
use std::sync::Arc;

pub(crate) fn shade(drive: u8, contrast: u8, raw: bool) -> u8 {
    if raw {
        return 255 - drive;
    }
    let gain = 0.5 + f32::from(contrast) / 63.;
    (222. - f32::from(drive) * 170. / 255. * gain).clamp(28., 222.) as u8
}

pub(crate) fn display(pixels: Arc<[u8; 6144]>, contrast: u8, raw: bool) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let dpi = window.scale_factor();
            let width = f32::from(bounds.size.width) * dpi;
            let height = f32::from(bounds.size.height) * dpi;
            let scale = (width / 96.).min(height / 64.).floor();
            if scale < 1. {
                return;
            }
            let origin = point(
                px((f32::from(bounds.origin.x) * dpi + (width - 96. * scale) / 2.).floor() / dpi),
                px((f32::from(bounds.origin.y) * dpi + (height - 64. * scale) / 2.).floor() / dpi),
            );
            let block = scale / dpi;
            for y in 0..64 {
                let mut x = 0;
                while x < 96 {
                    let value = shade(pixels[y * 96 + x], contrast, raw);
                    let mut end = x + 1;
                    while end < 96 && shade(pixels[y * 96 + end], contrast, raw) == value {
                        end += 1;
                    }
                    let gray = u32::from(value);
                    window.paint_quad(fill(
                        Bounds::new(
                            origin + point(px(x as f32 * block), px(y as f32 * block)),
                            size(px((end - x) as f32 * block), px(block)),
                        ),
                        rgb(gray * 0x010101),
                    ));
                    x = end;
                }
            }
        },
    )
    .size_full()
}

pub(crate) fn screenshot(pixels: &[u8; 6144], contrast: u8, raw: bool) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 96, 64);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()?
            .write_image_data(&pixels.map(|p| shade(p, contrast, raw)))?;
    }
    Ok(bytes)
}
