use crate::{motion::Motion, optical::Link, storage, Id, Worker};
use anyhow::{ensure, Context, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use hs_core::{Machine, Snapshot};
use std::path::{Path, PathBuf};

pub fn slot_path(eeprom: &Path, slot: u8) -> PathBuf {
    let mut directory = eeprom.as_os_str().to_owned();
    directory.push(".states");
    PathBuf::from(directory).join(format!("slot-{slot}.hstepstate"))
}

pub(crate) fn slots(eeprom: &Path) -> [bool; 10] {
    std::array::from_fn(|index| slot_path(eeprom, index as u8 + 1).is_file())
}

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
    pub(crate) fn encode(self) -> Result<SavedSession> {
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
            ensure!(session.motion.valid(), "Invalid walking cadence");
        }
        Ok(capture)
    }
}

impl SavedSession {
    fn decode(self) -> Result<PendingSession> {
        Ok(PendingSession {
            firmware: self.firmware,
            snapshot: Snapshot::decode(&self.machine)?,
            motion: self.motion,
            next_motion: self.next_motion,
            reset_release: self.reset_release,
        })
    }
}

pub(crate) struct RestorePoint {
    pub sessions: Vec<(Id, PendingSession)>,
    pub link: Option<Link>,
    pub paused: bool,
}

impl RestorePoint {
    pub fn contains(&self, id: Id) -> bool {
        self.sessions.iter().any(|(member, _)| *member == id)
    }
}

impl Worker {
    pub(crate) fn capture_point(&self, id: Id) -> Result<RestorePoint> {
        let link = self
            .links
            .iter()
            .find(|link| link.ids.contains(&id))
            .cloned();
        let ids = link
            .as_ref()
            .map(|link| link.ids.to_vec())
            .unwrap_or_else(|| vec![id]);
        let sessions = ids
            .into_iter()
            .map(|id| {
                Ok((
                    id,
                    self.sessions
                        .get(&id)
                        .context("Open firmware first")?
                        .capture_state(),
                ))
            })
            .collect::<Result<_>>()?;
        Ok(RestorePoint {
            sessions,
            link,
            paused: self.sessions[&id].paused,
        })
    }

    pub(crate) fn restore(&mut self, id: Id, mut capture: Capture) -> Result<()> {
        let ids = if capture.sessions.len() == 2 {
            self.links
                .iter()
                .find(|link| link.ids.contains(&id))
                .context("Connect the two players before loading their linked state")?
                .ids
                .to_vec()
        } else {
            vec![id]
        };
        let sessions = ids
            .iter()
            .copied()
            .zip(capture.sessions)
            .map(|(id, saved)| Ok((id, saved.decode()?)))
            .collect::<Result<Vec<_>>>()?;
        for (side, (member, saved)) in sessions.iter().enumerate() {
            let session = self.sessions.get(member).context("Player closed")?;
            ensure!(
                storage::hash(&session.firmware) == saved.firmware,
                "Save state belongs to different firmware"
            );
            if let Some(link) = &capture.link {
                let machine = Machine::from_snapshot(&saved.snapshot);
                ensure!(
                    Some(machine.now().raw())
                        == link.origins[side].raw().checked_add(link.elapsed.raw()),
                    "Invalid linked clock"
                );
                ensure!(
                    link.incoming[side]
                        .iter()
                        .all(|edge| edge.at >= link.elapsed),
                    "Invalid pending optical input"
                );
            }
        }
        if let Some(link) = &mut capture.link {
            link.ids = [ids[0], ids[1]];
        }
        let undo = self.capture_point(id)?;
        self.apply_point(
            id,
            RestorePoint {
                sessions,
                link: capture.link,
                paused: true,
            },
        )?;
        for (member, _) in &undo.sessions {
            self.forget_undo(*member);
        }
        self.undos.insert(id, undo);
        Ok(())
    }

    fn apply_point(&mut self, id: Id, point: RestorePoint) -> Result<()> {
        ensure!(
            point
                .sessions
                .iter()
                .all(|(member, _)| self.sessions.contains_key(member)),
            "A player in this state has closed"
        );
        self.disconnect(id)?;
        let linked = point.link.is_some();
        for (member, saved) in point.sessions {
            self.disconnect(member)?;
            self.sessions
                .get_mut(&member)
                .unwrap()
                .restore(saved, self.sound.rate, linked)?;
            self.sessions.get_mut(&member).unwrap().paused = point.paused;
        }
        if let Some(link) = point.link {
            self.links.push(link);
        }
        Ok(())
    }

    pub(crate) fn undo_load(&mut self, id: Id) -> Result<()> {
        let key = self
            .undos
            .iter()
            .find(|(_, point)| point.contains(id))
            .map(|(key, _)| *key)
            .context("No state load to undo")?;
        let point = self.undos.remove(&key).unwrap();
        self.apply_point(id, point)
    }

    pub(crate) fn forget_undo(&mut self, id: Id) {
        self.undos.retain(|_, point| !point.contains(id));
    }
}
