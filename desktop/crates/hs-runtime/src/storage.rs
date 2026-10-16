use anyhow::{ensure, Context, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use fs2::FileExt;
use hs_core::{Images, Machine};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const IMAGE_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, BorshSerialize, BorshDeserialize)]
pub(crate) struct Persistent {
    pub firmware_origin: [u8; 32],
    pub flash: Vec<u8>,
    pub eeprom: Vec<u8>,
    pub status: u8,
    pub sensor: Vec<u8>,
}

impl Persistent {
    pub fn capture(machine: &Machine, firmware_origin: [u8; 32]) -> Self {
        Self {
            firmware_origin,
            flash: machine.firmware().to_vec(),
            eeprom: machine.eeprom().to_vec(),
            status: machine.eeprom_status(),
            sensor: machine.sensor_nonvolatile().to_vec(),
        }
    }

    pub fn machine(&self, firmware: &[u8]) -> Result<Machine> {
        ensure!(
            self.firmware_origin == hash(firmware),
            "EEPROM companion belongs to different firmware"
        );
        Ok(Machine::new(Images {
            firmware: &self.flash,
            eeprom: &self.eeprom,
            eeprom_status: self.status,
            sensor_nonvolatile: Some(&self.sensor),
        })?)
    }

    fn companion(&self) -> Result<Vec<u8>> {
        pack(b"HSTEPnv1", self)
    }
}

#[derive(BorshSerialize, BorshDeserialize)]
struct Transaction {
    previous_eeprom: [u8; 32],
    previous_companion: Option<[u8; 32]>,
    next: Persistent,
}

pub(crate) struct Binding {
    pub path: PathBuf,
    expected: [u8; 32],
    companion: Option<[u8; 32]>,
    previous: Option<Persistent>,
}

pub(crate) fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(IMAGE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= IMAGE_LIMIT as usize, "File is too large");
    Ok(bytes)
}

fn adjacent(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub(crate) fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("File has no parent folder")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) fn pack<T: BorshSerialize>(magic: &[u8; 8], value: &T) -> Result<Vec<u8>> {
    let body = borsh::to_vec(value)?;
    let mut bytes = magic.to_vec();
    bytes.extend(hash(&body));
    bytes.extend(body);
    Ok(bytes)
}

pub(crate) fn unpack<T: BorshDeserialize>(magic: &[u8; 8], bytes: &[u8]) -> Result<T> {
    ensure!(
        bytes.len() >= 40 && &bytes[..8] == magic,
        "Unsupported file format"
    );
    ensure!(
        bytes[8..40] == hash(&bytes[40..]),
        "File checksum does not match"
    );
    Ok(borsh::from_slice(&bytes[40..])?)
}

impl Binding {
    pub fn open(path: &Path, create: bool, firmware: &[u8]) -> Result<(Self, Machine)> {
        let path = if create {
            let parent = path
                .parent()
                .context("EEPROM has no parent folder")?
                .canonicalize()?;
            parent.join(path.file_name().context("Choose an EEPROM filename")?)
        } else {
            path.canonicalize()?
        };
        if create {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(&[0xff; 65_536])?;
            file.sync_all()?;
        }
        let eeprom = read(&path)?;
        ensure!(eeprom.len() == 65_536, "EEPROM must contain 65,536 bytes");
        let companion_path = adjacent(&path, ".hachistep");
        let companion = optional(&companion_path)?;
        let mut binding = Self {
            path,
            expected: hash(&eeprom),
            companion: companion.as_deref().map(hash),
            previous: None,
        };
        if adjacent(&binding.path, ".hachistep.pending").exists() {
            let _lock = lock(&binding.path)?;
            binding.recover()?;
        }
        let eeprom = read(&binding.path)?;
        let machine = match optional(&companion_path)? {
            Some(bytes) => {
                let mut saved: Persistent = unpack(b"HSTEPnv1", &bytes)?;
                ensure!(saved.eeprom == eeprom, "EEPROM and companion do not match");
                saved.eeprom = eeprom;
                saved.machine(firmware)?
            }
            None => Machine::new(Images {
                firmware,
                eeprom: &eeprom,
                eeprom_status: 0,
                sensor_nonvolatile: None,
            })?,
        };
        binding.previous = Some(Persistent::capture(&machine, hash(firmware)));
        Ok((binding, machine))
    }

    fn recover(&mut self) -> Result<()> {
        if let Some(bytes) = optional(&adjacent(&self.path, ".hachistep.pending"))? {
            let transaction: Transaction = unpack(b"HSTEPtx1", &bytes)?;
            let next_companion = transaction.next.companion()?;
            let current = hash(&read(&self.path)?);
            let companion = optional(&adjacent(&self.path, ".hachistep"))?
                .as_deref()
                .map(hash);
            ensure!(
                current == transaction.previous_eeprom || current == hash(&transaction.next.eeprom),
                "EEPROM changed outside hachistep during a pending write"
            );
            ensure!(
                companion == transaction.previous_companion
                    || companion == Some(hash(&next_companion)),
                "EEPROM companion changed outside hachistep during a pending write"
            );
            atomic(&self.path, &transaction.next.eeprom)?;
            atomic(&adjacent(&self.path, ".hachistep"), &next_companion)?;
            self.expected = hash(&transaction.next.eeprom);
            self.companion = Some(hash(&next_companion));
            fs::remove_file(adjacent(&self.path, ".hachistep.pending"))?;
            self.previous = Some(transaction.next);
        }
        Ok(())
    }

    pub fn commit(&mut self, next: Persistent) -> Result<()> {
        let _lock = lock(&self.path)?;
        self.recover()?;
        let current = read(&self.path)?;
        let companion = optional(&adjacent(&self.path, ".hachistep"))?;
        ensure!(
            hash(&current) == self.expected && companion.as_deref().map(hash) == self.companion,
            "EEPROM changed outside hachistep. Use Save EEPROM As to keep this session"
        );
        let previous = match companion {
            Some(bytes) => bytes,
            None => self
                .previous
                .as_ref()
                .context("Missing previous EEPROM image")?
                .companion()?,
        };
        atomic(&adjacent(&self.path, ".hachistep.backup"), &previous)?;
        let transaction = Transaction {
            previous_eeprom: self.expected,
            previous_companion: self.companion,
            next,
        };
        atomic(
            &adjacent(&self.path, ".hachistep.pending"),
            &pack(b"HSTEPtx1", &transaction)?,
        )?;
        self.recover()
    }

    pub fn save_as(&mut self, path: &Path, next: Persistent, firmware: &[u8]) -> Result<()> {
        if path.canonicalize().ok().as_ref() == Some(&self.path) {
            return self.commit(next);
        }
        let (mut replacement, _) = Self::open(path, !path.exists(), firmware)?;
        replacement.commit(next)?;
        *self = replacement;
        Ok(())
    }
}

fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(adjacent(path, ".hachistep.lock"))?;
    file.try_lock_exclusive()
        .context("EEPROM is being saved by another player")?;
    Ok(file)
}

fn optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
