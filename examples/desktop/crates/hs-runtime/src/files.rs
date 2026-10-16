use crate::{
    motion::Motion,
    storage::{self, Binding, Persistent},
    Id,
};
use anyhow::{ensure, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use hs_core::{Machine, Snapshot};
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
};

#[derive(BorshSerialize, BorshDeserialize)]
pub(crate) struct Capture {
    pub revision: String,
    pub sessions: Vec<SavedSession>,
    pub link: Option<crate::optical::Link>,
}

#[derive(BorshSerialize, BorshDeserialize)]
pub(crate) struct SavedSession {
    pub firmware: [u8; 32],
    pub machine: Vec<u8>,
    pub motion: Motion,
    pub next_motion: hs_core::Time,
    pub reset_release: Option<hs_core::Time>,
}

pub(crate) struct PendingSession {
    pub firmware: [u8; 32],
    pub snapshot: Snapshot,
    pub motion: Motion,
    pub next_motion: hs_core::Time,
    pub reset_release: Option<hs_core::Time>,
}

impl PendingSession {
    fn encode(self) -> Result<SavedSession> {
        Ok(SavedSession {
            firmware: self.firmware,
            machine: self.snapshot.encode()?,
            motion: self.motion,
            next_motion: self.next_motion,
            reset_release: self.reset_release,
        })
    }
}

impl Capture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let capture: Self = storage::unpack(b"HSTEPst1", bytes)?;
        ensure!(
            capture.revision == crate::CORE_REVISION,
            "Save state uses a different core revision"
        );
        ensure!(
            (1..=2).contains(&capture.sessions.len()),
            "Invalid player count"
        );
        ensure!(
            capture.link.is_some() == (capture.sessions.len() == 2),
            "Invalid linked state"
        );
        for session in &capture.sessions {
            Snapshot::decode(&session.machine)?;
            ensure!(
                (30..=240).contains(&session.motion.cadence),
                "Invalid walking cadence"
            );
        }
        Ok(capture)
    }
}

pub(crate) enum Job {
    Open {
        id: Id,
        firmware: PathBuf,
        save: PathBuf,
        create: bool,
    },
    Commit {
        id: Id,
        image: Persistent,
    },
    SaveAs {
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
        path: PathBuf,
    },
    Saved {
        id: Id,
        path: PathBuf,
    },
    Written(Id),
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
            let mut bindings = std::collections::BTreeMap::<Id, (Binding, Arc<Vec<u8>>)>::new();
            for job in receive {
                let (id, operation, result): (Id, _, Result<Reply>) = match job {
                    Job::Open {
                        id,
                        firmware,
                        save,
                        create,
                    } => (
                        id,
                        "Open",
                        (|| {
                            let firmware = Arc::new(storage::read(&firmware)?);
                            ensure!(
                                firmware.len() == 49_152,
                                "Firmware must contain 49,152 bytes"
                            );
                            let (binding, machine) = Binding::open(&save, create, &firmware)?;
                            let path = binding.path.clone();
                            bindings.insert(id, (binding, firmware.clone()));
                            Ok(Reply::Opened {
                                id,
                                machine: Box::new(machine),
                                firmware,
                                path,
                            })
                        })(),
                    ),
                    Job::Commit { id, image } => (
                        id,
                        "Save data",
                        (|| {
                            let (binding, _) = bindings
                                .get_mut(&id)
                                .ok_or_else(|| anyhow::anyhow!("Player closed"))?;
                            binding.commit(image)?;
                            Ok(Reply::Saved {
                                id,
                                path: binding.path.clone(),
                            })
                        })(),
                    ),
                    Job::SaveAs { id, path, image } => (
                        id,
                        "Save data",
                        (|| {
                            let (binding, firmware) = bindings
                                .get_mut(&id)
                                .ok_or_else(|| anyhow::anyhow!("Player closed"))?;
                            binding.save_as(&path, image, firmware)?;
                            Ok(Reply::Saved {
                                id,
                                path: binding.path.clone(),
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
                            storage::atomic(&path, &storage::pack(b"HSTEPst1", &capture)?)?;
                            Ok(Reply::Written(id))
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
