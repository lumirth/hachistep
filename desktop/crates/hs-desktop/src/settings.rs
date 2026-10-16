use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    thread,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Recent {
    pub firmware: PathBuf,
    pub eeprom: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub appearance: usize,
    pub palette: crate::lcd::Palette,
    pub settings_page: usize,
    pub keys: [String; 3],
    pub gamepad: [usize; 3],
    pub mute: bool,
    pub speed: u16,
    pub turbo: u16,
    pub turbo_key: String,
    pub slots: BTreeMap<PathBuf, u8>,
    pub recent: Vec<Recent>,
    pub recent_firmware: Vec<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: 0,
            palette: crate::lcd::Palette::default(),
            settings_page: 0,
            keys: ["a".into(), "s".into(), "d".into()],
            gamepad: [0, 1, 2],
            mute: false,
            speed: 100,
            turbo: 400,
            turbo_key: "space".into(),
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
        if settings.keys.iter().any(String::is_empty) {
            settings.keys = ["a".into(), "s".into(), "d".into()];
        }
        settings
            .slots
            .values_mut()
            .for_each(|slot| *slot = (*slot).clamp(1, 10));
        if settings.turbo_key.is_empty() {
            settings.turbo_key = "space".into();
        }
        settings.appearance = settings.appearance.min(2);
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
        (settings, send)
    }
    pub fn remember(&mut self, pair: Recent) {
        self.remember_firmware(pair.firmware.clone());
        self.recent
            .retain(|existing| existing.eeprom != pair.eeprom);
        self.recent.insert(0, pair);
        self.recent.truncate(12);
    }
    pub fn remember_firmware(&mut self, path: PathBuf) {
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

pub(crate) const GAMEPAD: [(hs_runtime::GamepadButton, &str); 8] = [
    (hs_runtime::GamepadButton::DPadLeft, "D-pad left"),
    (hs_runtime::GamepadButton::South, "South"),
    (hs_runtime::GamepadButton::DPadRight, "D-pad right"),
    (hs_runtime::GamepadButton::East, "East"),
    (hs_runtime::GamepadButton::West, "West"),
    (hs_runtime::GamepadButton::North, "North"),
    (hs_runtime::GamepadButton::LeftTrigger, "Left shoulder"),
    (hs_runtime::GamepadButton::RightTrigger, "Right shoulder"),
];
