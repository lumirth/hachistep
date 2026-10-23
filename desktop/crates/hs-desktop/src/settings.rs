use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    thread,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Recent {
    pub firmware: PathBuf,
    pub eeprom: PathBuf,
    #[serde(default)]
    pub firmware_hash: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SavedPalette {
    pub id: String,
    pub name: String,
    pub colors: crate::lcd::Palette,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WalkerPreferences {
    pub palette: Option<String>,
    pub speed: Option<u16>,
    pub turbo: Option<u16>,
    pub cadence: Option<u16>,
    pub muted: Option<bool>,
    pub raw: bool,
    pub status: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub appearance: usize,
    pub palette: crate::lcd::Palette,
    pub palette_id: String,
    pub lcd_margin: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_palette: Option<crate::lcd::Palette>,
    pub palettes: Vec<SavedPalette>,
    pub palettes_seeded: bool,
    pub default_firmware: Option<crate::firmware::Firmware>,
    pub player_bounds: Option<[f32; 4]>,
    pub settings_page: usize,
    pub keys: [String; 3],
    pub gamepad: [usize; 3],
    pub mute: bool,
    pub speed: u16,
    pub turbo: u16,
    pub turbo_key: String,
    pub walk_key: String,
    pub cadence: u16,
    pub hotkeys: BTreeMap<String, String>,
    pub walkers: BTreeMap<PathBuf, WalkerPreferences>,
    pub slots: BTreeMap<PathBuf, u8>,
    pub recent: Vec<Recent>,
    pub recent_firmware: Vec<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: 0,
            palette: crate::lcd::Palette::default(),
            palette_id: String::new(),
            lcd_margin: 0,
            custom_palette: None,
            palettes: Vec::new(),
            palettes_seeded: false,
            default_firmware: None,
            player_bounds: None,
            settings_page: 0,
            keys: ["a".into(), "s".into(), "d".into()],
            gamepad: [0, 1, 2],
            mute: false,
            speed: 100,
            turbo: 400,
            turbo_key: "space".into(),
            walk_key: "w".into(),
            cadence: 120,
            hotkeys: BTreeMap::new(),
            walkers: BTreeMap::new(),
            slots: BTreeMap::new(),
            recent: Vec::new(),
            recent_firmware: Vec::new(),
        }
    }
}

impl Preferences {
    pub fn load() -> (Self, Sender<Self>) {
        let path =
            ProjectDirs::from("", "", "hachistep").map(|p| p.config_dir().join("settings.json"));
        let mut settings: Self = path
            .as_ref()
            .and_then(|p| fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if settings.palette_id.is_empty() {
            settings.palette_id = crate::lcd::PRESETS
                .iter()
                .find(|(_, colors)| *colors == settings.palette)
                .map(|(name, _)| name.to_string())
                .unwrap_or_else(|| "custom".into());
            if settings.palette_id == "custom" {
                settings.custom_palette = Some(settings.palette);
            }
        }
        let legacy = settings
            .custom_palette
            .take()
            .or_else(|| (settings.palette_id == "custom").then_some(settings.palette));
        if let Some(colors) = legacy {
            if !settings
                .palettes
                .iter()
                .any(|palette| palette.id == "custom")
            {
                let mut name = "Custom".to_string();
                let mut suffix = 2;
                while settings.palettes.iter().any(|palette| palette.name == name) {
                    name = format!("Custom {suffix}");
                    suffix += 1;
                }
                settings.palettes.push(SavedPalette {
                    id: "custom".into(),
                    name,
                    colors,
                });
            }
        }
        if !settings.palettes_seeded {
            let saved = std::mem::take(&mut settings.palettes);
            let mut seeded: Vec<_> = crate::lcd::PRESETS
                .iter()
                .filter(|(id, _)| !saved.iter().any(|palette| palette.id == *id))
                .map(|(name, colors)| SavedPalette {
                    id: (*name).into(),
                    name: (*name).into(),
                    colors: *colors,
                })
                .collect();
            seeded.extend(saved);
            settings.palettes = seeded;
            settings.palettes_seeded = true;
        }
        if settings.palettes.is_empty() {
            let (name, colors) = crate::lcd::PRESETS[1];
            settings.palettes.push(SavedPalette {
                id: name.into(),
                name: name.into(),
                colors,
            });
        }
        settings.disambiguate_palette_names();
        let selected = settings.palette_id.clone();
        if settings.palette_named(&selected).is_some() {
            settings.select_palette(&selected);
        } else {
            let fallback = settings.palettes[0].id.clone();
            settings.select_palette(&fallback);
        }
        for recent in &mut settings.recent {
            if recent.firmware_hash.is_none() {
                recent.firmware_hash = crate::firmware::identify(&recent.firmware).ok();
            }
        }
        if settings.keys.iter().any(String::is_empty) {
            settings.keys = ["a".into(), "s".into(), "d".into()];
        }
        settings
            .slots
            .values_mut()
            .for_each(|slot| *slot = (*slot).clamp(1, 10));
        if !(30..=240).contains(&settings.cadence) {
            settings.cadence = 120;
        }
        settings.appearance = settings.appearance.min(2);
        settings.lcd_margin = settings.lcd_margin.min(128);
        if !(25..=800).contains(&settings.speed) {
            settings.speed = 100;
        }
        if !(100..=800).contains(&settings.turbo) {
            settings.turbo = 400;
        }
        let (send, receive) = mpsc::channel::<Self>();
        thread::Builder::new()
            .name("hachistep-preferences".into())
            .spawn(move || {
                for settings in receive {
                    if let Some(path) = &path {
                        let result = (|| -> anyhow::Result<()> {
                            fs::create_dir_all(path.parent().unwrap())?;
                            let bytes = serde_json::to_vec_pretty(&settings)?;
                            let mut temporary =
                                tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
                            temporary.write_all(&bytes)?;
                            temporary.as_file().sync_all()?;
                            temporary.persist(path).map_err(|error| error.error)?;
                            Ok(())
                        })();
                        if let Err(error) = result {
                            eprintln!("Could not save settings: {error:#}");
                        }
                    }
                }
            })
            .expect("create preferences worker");
        if settings.recent_firmware.is_empty() {
            for pair in settings.recent.clone().into_iter().rev() {
                settings.remember_firmware(pair.firmware);
            }
        }
        settings
            .recent_firmware
            .retain(|path| !crate::firmware::is_retained(path));
        (settings, send)
    }
    pub fn palette_named(&self, id: &str) -> Option<crate::lcd::Palette> {
        self.palettes.iter().find(|p| p.id == id).map(|p| p.colors)
    }
    pub fn add_palette(&mut self, name: String, colors: crate::lcd::Palette) -> String {
        let base = format!(
            "user-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut id = base.clone();
        let mut suffix = 2;
        while self.palettes.iter().any(|palette| palette.id == id) {
            id = format!("{base}-{suffix}");
            suffix += 1;
        }
        let mut unique = name.clone();
        let mut suffix = 2;
        while self.palettes.iter().any(|palette| palette.name == unique) {
            unique = format!("{name} {suffix}");
            suffix += 1;
        }
        self.palettes.push(SavedPalette {
            id: id.clone(),
            name: unique,
            colors,
        });
        id
    }
    pub fn default_palettes_changed(&self) -> bool {
        crate::lcd::PRESETS.iter().any(|(id, colors)| {
            self.palettes
                .iter()
                .find(|palette| palette.id == *id)
                .is_none_or(|palette| palette.name != *id || palette.colors != *colors)
        })
    }
    pub fn restore_default_palettes(&mut self) {
        for (name, colors) in crate::lcd::PRESETS {
            if let Some(palette) = self.palettes.iter_mut().find(|palette| palette.id == name) {
                palette.name = name.into();
                palette.colors = colors;
            } else {
                self.palettes.push(SavedPalette {
                    id: name.into(),
                    name: name.into(),
                    colors,
                });
            }
        }
        self.disambiguate_palette_names();
        let selected = self.palette_id.clone();
        self.select_palette(&selected);
    }
    fn disambiguate_palette_names(&mut self) {
        let mut order: Vec<_> = (0..self.palettes.len()).collect();
        order.sort_by_key(|&index| {
            let palette = &self.palettes[index];
            !crate::lcd::PRESETS
                .iter()
                .any(|(id, _)| palette.id == *id && palette.name == *id)
        });
        let mut names = BTreeSet::new();
        for index in order {
            let palette = &mut self.palettes[index];
            let base = palette.name.clone();
            let mut suffix = 2;
            while !names.insert(palette.name.clone()) {
                palette.name = format!("{base} {suffix}");
                suffix += 1;
            }
        }
    }
    pub fn select_palette(&mut self, id: &str) {
        if let Some(colors) = self.palette_named(id) {
            self.palette = colors;
            self.palette_id = id.to_owned();
        }
    }
    pub fn remember(&mut self, pair: Recent) {
        self.remember_firmware(pair.firmware.clone());
        self.recent
            .retain(|existing| existing.eeprom != pair.eeprom);
        self.recent.insert(0, pair);
        self.recent.truncate(12);
    }
    pub fn remember_firmware(&mut self, path: PathBuf) {
        if crate::firmware::is_retained(&path) {
            return;
        }
        self.recent_firmware.retain(|existing| existing != &path);
        self.recent_firmware.insert(0, path);
        self.recent_firmware.truncate(12);
    }
}

pub(crate) fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn key_name(key: &str) -> String {
    gpui_kit::Keystroke::parse(key)
        .map(|key| {
            let label = key.to_string();
            if key
                .key
                .strip_prefix('f')
                .is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)))
            {
                label.replacen(&key.key, &key.key.to_uppercase(), 1)
            } else {
                label
            }
        })
        .unwrap_or_else(|_| key.to_owned())
}

pub(crate) fn recent_label(path: &Path, others: &[&Path]) -> String {
    for parent in path.ancestors().skip(1) {
        if let Ok(suffix) = path.strip_prefix(parent) {
            if others
                .iter()
                .filter(|other| other.ends_with(suffix))
                .count()
                <= 1
            {
                return suffix.display().to_string();
            }
        }
    }
    path.display().to_string()
}

pub(crate) const GAMEPAD: [(hs_runtime::GamepadButton, &str); 10] = [
    (hs_runtime::GamepadButton::DPadLeft, "D-pad left"),
    (hs_runtime::GamepadButton::South, "South"),
    (hs_runtime::GamepadButton::DPadRight, "D-pad right"),
    (hs_runtime::GamepadButton::East, "East"),
    (hs_runtime::GamepadButton::West, "West"),
    (hs_runtime::GamepadButton::North, "North"),
    (hs_runtime::GamepadButton::LeftTrigger, "Left shoulder"),
    (hs_runtime::GamepadButton::RightTrigger, "Right shoulder"),
    (hs_runtime::GamepadButton::DPadUp, "D-pad up"),
    (hs_runtime::GamepadButton::DPadDown, "D-pad down"),
];
