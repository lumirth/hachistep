use crate::{
    state::{self, Capture, PendingSession},
    storage::{self, Binding, Persistent},
    Id,
};
use anyhow::{ensure, Result};
use hs_core::Machine;
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
};

pub(crate) enum Job {
    Open {
        id: Id,
        firmware: PathBuf,
        eeprom: Option<PathBuf>,
    },
    Commit {
        id: Id,
        image: Persistent,
    },
    SaveEepromAs {
        id: Id,
        path: PathBuf,
        image: Persistent,
    },
    Write {
        id: Id,
        path: PathBuf,
        bytes: Vec<u8>,
    },
    Load {
        id: Id,
        path: PathBuf,
    },
    Capture {
        id: Id,
        path: PathBuf,
        sessions: Vec<PendingSession>,
        link: Option<crate::optical::Link>,
    },
    Close(Id),
}

pub(crate) enum Reply {
    Opened {
        id: Id,
        machine: Box<Machine>,
        firmware: Arc<Vec<u8>>,
        path: Option<PathBuf>,
        slots: [bool; 10],
    },
    Saved {
        id: Id,
        path: PathBuf,
        slots: [bool; 10],
    },
    Written(Id),
    StateWritten {
        id: Id,
        slots: [bool; 10],
    },
    Loaded {
        id: Id,
        capture: Capture,
    },
    Failed {
        id: Id,
        operation: &'static str,
        error: String,
    },
    Closed(Id),
}

pub(crate) struct Files(Sender<Job>);

impl Files {
    pub fn send(&self, job: Job) -> Result<()> {
        self.0
            .send(job)
            .map_err(|_| anyhow::anyhow!("File worker stopped"))
    }
}

pub(crate) fn start() -> (Files, Receiver<Reply>) {
    let (send, receive) = mpsc::channel();
    let (responses, replies) = mpsc::channel();
    thread::Builder::new()
        .name("hachistep-files".into())
        .spawn(move || {
            let mut bindings =
                std::collections::BTreeMap::<Id, (Option<Binding>, Arc<Vec<u8>>)>::new();
            for job in receive {
                let (id, operation, result): (Id, _, Result<Reply>) = match job {
                    Job::Open {
                        id,
                        firmware,
                        eeprom,
                    } => (
                        id,
                        "Open",
                        (|| {
                            let firmware = Arc::new(storage::read(&firmware)?);
                            ensure!(
                                firmware.len() == 49_152,
                                "Firmware must contain 49,152 bytes"
                            );
                            let (binding, machine) = match eeprom {
                                Some(path) => {
                                    let (binding, machine) =
                                        Binding::open(&path, false, &firmware)?;
                                    (Some(binding), machine)
                                }
                                None => (
                                    None,
                                    Machine::new(hs_core::Images {
                                        firmware: &firmware,
                                        eeprom: &[0xff; 65_536],
                                        eeprom_status: 0,
                                        sensor_nonvolatile: None,
                                    })?,
                                ),
                            };
                            let path = binding.as_ref().map(|binding| binding.path.clone());
                            let slots = path
                                .as_ref()
                                .map(|path| state::slots(path))
                                .unwrap_or([false; 10]);
                            bindings.insert(id, (binding, firmware.clone()));
                            Ok(Reply::Opened {
                                id,
                                machine: Box::new(machine),
                                firmware,
                                path,
                                slots,
                            })
                        })(),
                    ),
                    Job::Commit { id, image } => (
                        id,
                        "EEPROM",
                        (|| {
                            let (binding, _) = bindings
                                .get_mut(&id)
                                .ok_or_else(|| anyhow::anyhow!("Player closed"))?;
                            let binding = binding.as_mut().ok_or_else(|| {
                                anyhow::anyhow!("Choose an EEPROM filename first")
                            })?;
                            binding.commit(image)?;
                            Ok(Reply::Saved {
                                id,
                                path: binding.path.clone(),
                                slots: state::slots(&binding.path),
                            })
                        })(),
                    ),
                    Job::SaveEepromAs { id, path, image } => (
                        id,
                        "EEPROM",
                        (|| {
                            let canonical = path.canonicalize().ok();
                            ensure!(
                                !bindings.iter().any(|(other, (binding, _))| *other != id
                                    && binding.as_ref().is_some_and(
                                        |binding| Some(&binding.path) == canonical.as_ref()
                                    )),
                                "EEPROM is open in another player"
                            );
                            let (binding, firmware) = bindings
                                .get_mut(&id)
                                .ok_or_else(|| anyhow::anyhow!("Player closed"))?;
                            if let Some(binding) = binding {
                                binding.save_as(&path, image, firmware)?;
                            } else {
                                let (mut replacement, _) =
                                    Binding::open(&path, !path.exists(), firmware)?;
                                replacement.commit(image)?;
                                *binding = Some(replacement);
                            }
                            let binding = binding.as_ref().unwrap();
                            Ok(Reply::Saved {
                                id,
                                path: binding.path.clone(),
                                slots: state::slots(&binding.path),
                            })
                        })(),
                    ),
                    Job::Write { id, path, bytes } => (
                        id,
                        "Write file",
                        storage::atomic(&path, &bytes).map(|()| Reply::Written(id)),
                    ),
                    Job::Load { id, path } => (
                        id,
                        "Load state",
                        storage::read(&path)
                            .and_then(|bytes| Capture::decode(&bytes))
                            .map(|capture| Reply::Loaded { id, capture }),
                    ),
                    Job::Capture {
                        id,
                        path,
                        sessions,
                        link,
                    } => (
                        id,
                        "Save state",
                        (|| {
                            let sessions = sessions
                                .into_iter()
                                .map(PendingSession::encode)
                                .collect::<Result<Vec<_>>>()?;
                            let capture = Capture {
                                revision: crate::CORE_REVISION.into(),
                                sessions,
                                link,
                            };
                            if let Some(parent) = path.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            storage::atomic(&path, &storage::pack(b"HSTEPst2", &capture)?)?;
                            let slots = bindings
                                .get(&id)
                                .and_then(|(binding, _)| binding.as_ref())
                                .map(|binding| state::slots(&binding.path))
                                .unwrap_or([false; 10]);
                            Ok(Reply::StateWritten { id, slots })
                        })(),
                    ),
                    Job::Close(id) => {
                        bindings.remove(&id);
                        (id, "Close", Ok(Reply::Closed(id)))
                    }
                };
                let reply = result.unwrap_or_else(|error| Reply::Failed {
                    id,
                    operation,
                    error: format!("{error:#}"),
                });
                if responses.send(reply).is_err() {
                    break;
                }
            }
        })
        .expect("create file worker");
    (Files(send), replies)
}
