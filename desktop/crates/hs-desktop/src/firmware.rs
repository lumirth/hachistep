use anyhow::{ensure, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Firmware {
    pub name: String,
    pub hash: String,
}

pub(crate) fn identify(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("Could not open {}", path.display()))?;
    ensure!(bytes.len() == 49_152, "Firmware must contain 49,152 bytes");
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn directory() -> Result<PathBuf> {
    Ok(ProjectDirs::from("", "", "hachistep")
        .context("Application storage is unavailable")?
        .data_local_dir()
        .join("firmware"))
}

pub(crate) fn is_retained(path: &Path) -> bool {
    directory().is_ok_and(|directory| path.parent() == Some(directory.as_path()))
}

pub(crate) fn retained(hash: &str) -> Option<PathBuf> {
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let path = directory().ok()?.join(format!("{hash}.bin"));
    (identify(&path).ok().as_deref() == Some(hash)).then_some(path)
}

impl Firmware {
    pub fn retain(path: &Path) -> Result<Self> {
        let hash = identify(path)?;
        let directory = directory()?;
        fs::create_dir_all(&directory)?;
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        let bytes = fs::read(path)?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == hash,
            "Firmware changed while opening"
        );
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(directory.join(format!("{hash}.bin")))
            .map_err(|e| e.error)?;
        Ok(Self {
            name: crate::settings::filename(path),
            hash,
        })
    }
    pub fn path(&self) -> Option<PathBuf> {
        retained(&self.hash)
    }
}
