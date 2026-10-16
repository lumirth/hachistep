use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Palette {
    pub panel: u32,
    pub ink: u32,
}

impl Default for Palette {
    fn default() -> Self {
        PRESETS[1].1
    }
}

pub(crate) const PRESETS: [(&str, Palette); 4] = [
    (
        "Classic",
        Palette {
            panel: 0xccd1c2,
            ink: 0x303a32,
        },
    ),
    (
        "Neutral",
        Palette {
            panel: 0xecedea,
            ink: 0x242829,
        },
    ),
    (
        "Olive",
        Palette {
            panel: 0xe9e2bf,
            ink: 0x4c5235,
        },
    ),
    (
        "Amber",
        Palette {
            panel: 0xf7e2af,
            ink: 0x764200,
        },
    ),
];

impl Palette {
    pub fn colors(self, contrast: u8, raw: bool) -> [u32; 256] {
        let linear = |value: f32| {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let srgb = |value: f32| {
            if value <= 0.0031308 {
                value * 12.92
            } else {
                1.055 * value.powf(1. / 2.4) - 0.055
            }
        };
        let channel = |color: u32, shift: u32| linear(((color >> shift) & 255u32) as f32 / 255.);
        std::array::from_fn(|drive| {
            let strength = (drive as f32 / 255.
                * if raw {
                    1.
                } else {
                    0.5 + f32::from(contrast) / 63.
                })
            .min(1.);
            [16, 8, 0].into_iter().fold(0, |color, shift| {
                let panel = channel(self.panel, shift);
                let ink = channel(self.ink, shift);
                color | ((srgb(panel + (ink - panel) * strength) * 255.).round() as u32) << shift
            })
        })
    }
}

pub(crate) fn scale(available: Size<Pixels>, dpi: f32) -> f32 {
    (f32::from(available.width) * dpi / 96.)
        .min(f32::from(available.height) * dpi / 64.)
        .floor()
}

pub(crate) fn dimensions(scale: f32, dpi: f32) -> Size<Pixels> {
    size(px(96. * scale / dpi), px(64. * scale / dpi))
}

pub(crate) fn display(
    pixels: Arc<[u8; 6144]>,
    contrast: u8,
    raw: bool,
    palette: Palette,
) -> impl IntoElement {
    let colors = palette.colors(contrast, raw);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let dpi = window.scale_factor();
            let width = f32::from(bounds.size.width) * dpi;
            let height = f32::from(bounds.size.height) * dpi;
            let scale = scale(bounds.size, dpi);
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
                    let value = pixels[y * 96 + x];
                    let mut end = x + 1;
                    while end < 96 && pixels[y * 96 + end] == value {
                        end += 1;
                    }
                    window.paint_quad(fill(
                        Bounds::new(
                            origin + point(px(x as f32 * block), px(y as f32 * block)),
                            size(px((end - x) as f32 * block), px(block)),
                        ),
                        rgb(colors[usize::from(value)]),
                    ));
                    x = end;
                }
            }
        },
    )
    .size_full()
}

pub(crate) fn screenshot(
    pixels: &[u8; 6144],
    contrast: u8,
    raw: bool,
    palette: Palette,
) -> anyhow::Result<Vec<u8>> {
    let colors = palette.colors(contrast, raw);
    let image: Vec<_> = pixels
        .iter()
        .flat_map(|&p| {
            let color = colors[usize::from(p)];
            [(color >> 16) as u8, (color >> 8) as u8, color as u8]
        })
        .collect();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 96, 64);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&image)?;
    }
    Ok(bytes)
}
