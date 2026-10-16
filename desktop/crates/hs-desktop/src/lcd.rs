use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Palette {
    pub panel: u32,
    pub ink: u32,
    #[serde(default)]
    pub middle: Option<[u32; 2]>,
}

impl Default for Palette {
    fn default() -> Self {
        PRESETS[1].1
    }
}

const fn four(colors: [u32; 4]) -> Palette {
    Palette {
        panel: colors[0],
        ink: colors[3],
        middle: Some([colors[1], colors[2]]),
    }
}

pub(crate) const PRESETS: [(&str, Palette); 8] = [
    ("Greyscale", four([0xffffff, 0xaaaaaa, 0x555555, 0x000000])),
    (
        "Greyscale 2",
        four([0xeaeaea, 0xa4a4a4, 0x5d5d5d, 0x171717]),
    ),
    (
        "Original Walker",
        four([0xb7b8b0, 0x808173, 0x666559, 0x1f1a17]),
    ),
    ("Poketch", four([0x70b070, 0x508050, 0x385030, 0x102818])),
    ("Pea Soup", four([0xc2c263, 0x6b6b18, 0x30320b, 0x121900])),
    ("DMG", four([0xc2c263, 0x5c6b18, 0x17320b, 0x021900])),
    (
        "Olive",
        Palette {
            panel: 0xe9e2bf,
            ink: 0x4c5235,
            middle: None,
        },
    ),
    (
        "Amber",
        Palette {
            panel: 0xf7e2af,
            ink: 0x764200,
            middle: None,
        },
    ),
];

fn blend(a: u32, b: u32, amount: f32) -> u32 {
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let srgb = |v: f32| {
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        }
    };
    [16, 8, 0].into_iter().fold(0, |color, shift| {
        let channel = |c| linear(((c >> shift) & 255u32) as f32 / 255.);
        color
            | ((srgb(channel(a) + (channel(b) - channel(a)) * amount) * 255.).round() as u32)
                << shift
    })
}

impl Palette {
    pub fn shades(self) -> [u32; 4] {
        let [light, dark] = self.middle.unwrap_or_else(|| {
            [
                blend(self.panel, self.ink, 1. / 3.),
                blend(self.panel, self.ink, 2. / 3.),
            ]
        });
        [self.panel, light, dark, self.ink]
    }
    pub fn with_color(mut self, ix: usize, value: u32) -> Self {
        match ix {
            0 => self.panel = value,
            3 => self.ink = value,
            _ => {
                let shades = self.shades();
                let middle = self.middle.get_or_insert([shades[1], shades[2]]);
                middle[ix - 1] = value;
            }
        }
        self
    }
    pub fn colors(self, contrast: u8, raw: bool) -> [u32; 256] {
        let shades = self.shades();
        std::array::from_fn(|drive| {
            let strength = (drive as f32 / 255.
                * if raw {
                    1.
                } else {
                    0.5 + f32::from(contrast) / 63.
                })
            .min(1.);
            if self.middle.is_none() {
                return blend(self.panel, self.ink, strength);
            }
            let position = strength * 3.;
            let ix = (position as usize).min(2);
            blend(shades[ix], shades[ix + 1], position - ix as f32)
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
