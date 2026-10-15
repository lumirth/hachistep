//! Save state file envelope and captured hardware state. Restoration validates
//! each component and reconstructs caches before exposing a machine.
use super::*;
use crate::{
    cpu::state::SavedCpu,
    state::{future, require},
};
use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"HSTEPST\0";
const HEADER: usize = 44;
// 384 flash pages * (128 cells * 8 bits * 8 charge bytes + 2 address
// bytes), the fixed RAM/EEPROM/LCD arrays, and bounded scalar owner records.
const MAX_PAYLOAD: usize = 4 * 1024 * 1024;

#[derive(BorshSerialize, BorshDeserialize)]
struct Saved {
    firmware_origin: [u8; 32],
    now: Time,
    last_effect: Time,
    cpu: SavedCpu,
    mcu: Mcu,
    eeprom: M95512,
    sensor: Bma150,
    lcd: Nt7508,
    conditions: Conditions,
    analog_pins: [Option<u16>; 7],
    pending: Option<Pending>,
    resume_after: Option<Resume>,
    serial: SerialLevels,
    piezo: Piezo,
    incident_light: bool,
    emitting: bool,
    reset_asserted: bool,
    reset_release: Option<ClockWait>,
    watchdog_reset: Option<ClockWait>,
    connected: bool,
    power: Power,
    stopped_by_core_fault: bool,
}
impl Saved {
    fn capture(m: &Machine) -> Result<Self, Error> {
        // Express lazy counters at the observation point on an owned copy.
        // The file describes hardware phase independently of earlier sync calls.
        let mut mcu = m.board.mcu.clone();
        require(
            !mcu.sync(m.board.observation_time(), &mut ())?,
            "unprocessed reset at capture",
        )?;
        let mut sensor = m.board.sensor.clone();
        sensor.sync_local_until(m.board.observation_time(), true)?;
        Ok(Self {
            firmware_origin: m.board.firmware_origin,
            now: m.board.now,
            // Only whether effects at `now` itself are done is state.
            last_effect: m.board.observation_time(),
            cpu: m.cpu.save()?,
            mcu,
            eeprom: m.board.eeprom.clone(),
            sensor,
            lcd: m.board.lcd.clone(),
            conditions: m.board.conditions,
            analog_pins: m.board.analog_pins,
            pending: m.board.pending.map(|mut p| {
                if p.lane == 0 {
                    p.high = 0;
                }
                p
            }),
            resume_after: m.board.resume_after,
            serial: m.board.serial,
            piezo: m.board.piezo,
            incident_light: m.board.incident_light,
            emitting: m.board.emitting,
            reset_asserted: m.board.reset_asserted,
            reset_release: m.board.reset_release,
            watchdog_reset: m.board.watchdog_reset,
            connected: m.board.connected,
            power: m.board.power.clone(),
            stopped_by_core_fault: m.board.fault.is_some(),
        })
    }
    fn restore(self) -> Result<Machine, Error> {
        let cpu = self.cpu.restore(self.stopped_by_core_fault)?;
        let mut m = Board {
            firmware_origin: self.firmware_origin,
            now: self.now,
            last_effect: self.last_effect,
            mcu: self.mcu,
            eeprom: self.eeprom,
            sensor: self.sensor,
            lcd: self.lcd,
            conditions: self.conditions,
            analog_pins: self.analog_pins,
            pending: self.pending,
            resume_after: self.resume_after,
            next_devices: None,
            appointments: Appointments::default(),
            board_appointments: [None; 7],
            changed_peripherals: 0,
            changed_board: 0,
            serial: self.serial,
            piezo: self.piezo,
            incident_light: self.incident_light,
            emitting: self.emitting,
            reset_asserted: self.reset_asserted,
            reset_release: self.reset_release,
            watchdog_reset: self.watchdog_reset,
            connected: self.connected,
            power: self.power,
            fault: self
                .stopped_by_core_fault
                .then_some(Error::Snapshot("saved session was stopped by a core fault")),
            stats: Statistics::default(),
            shift: Default::default(),
            late_output: LateOutput::default(),
        };
        require(
            m.last_effect <= m.now
                && m.conditions.avcc_override_millivolts != Some(0)
                && m.conditions.clocks == m.mcu.clocks.frequencies
                && m.power.rail
                    == if m.connected {
                        m.conditions.supply_millivolts
                    } else {
                        0
                    },
            "invalid saved board state",
        )?;
        m.power.validate(m.now)?;
        m.mcu.validate(m.now, m.fault.is_some())?;
        m.eeprom.validate(m.now)?;
        m.sensor.validate(m.now)?;
        m.lcd.validate(m.now)?;
        if let Some(p) = m.pending {
            p.wait.validate()?;
            let action = cpu.issued_action();
            require(
                p.wait.uses(Tap::cpu()) && action == Some(p.action),
                "saved request differs from CPU progress",
            )?;
            let split = match p.action {
                Action::Read { address, width, .. } | Action::Write { address, width, .. } => {
                    width == Width::Word && !Mcu::native_word(address & !1)
                }
                Action::Idle(_) => false,
                Action::Sleep => return Err(Error::Snapshot("sleep cannot be a timed bus action")),
            };
            require(
                p.split == split && p.lane <= u8::from(split) && (p.lane != 0 || p.high == 0),
                "invalid saved bus lane",
            )?;
            future(p.wait.deadline(&m.mcu.clocks)?, m.now)?;
        }
        if let Some(resume) = m.resume_after {
            let tap = match resume {
                Resume::Sleep(_) => Tap::cpu(),
                Resume::Wake { .. } => Tap::oscillator(),
            };
            resume.wait().validate()?;
            require(
                resume.wait().uses(tap) && cpu.sleeping() && m.pending.is_none(),
                "invalid saved wake progress",
            )?;
            future(resume.wait().deadline(&m.mcu.clocks)?, m.now)?;
        }
        for (wait, tap) in [
            (m.reset_release, Tap::system(1)),
            (m.watchdog_reset, Tap::on_chip(1)),
        ] {
            if let Some(wait) = wait {
                wait.validate()?;
                require(wait.uses(tap), "invalid reset clock obligation")?;
                future(wait.deadline(&m.mcu.clocks)?, m.now)?;
            }
        }
        // These are pure projections. Board resolution would deliver edges and
        // chip-select effects again, and must never be used as a load helper.
        m.refresh_deadline()?;
        Ok(Machine { cpu, board: m })
    }
}
impl Snapshot {
    /// Maximum native file size, for bounded frontend reads.
    pub const MAX_ENCODED_SIZE: usize = HEADER + MAX_PAYLOAD;

    /// Immutable identity of the construction firmware, before guest flash writes.
    pub fn firmware_origin(&self) -> [u8; 32] {
        self.state.board.firmware_origin
    }

    /// Encode the state needed to reproduce subsequent hardware behavior.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let payload = borsh::to_vec(&Saved::capture(&self.state)?)
            .map_err(|_| Error::Snapshot("cannot encode saved state"))?;
        require(
            payload.len() <= MAX_PAYLOAD,
            "saved state exceeds hardware storage bound",
        )?;
        let mut bytes = Vec::with_capacity(HEADER + payload.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&Sha256::digest(&payload));
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }
    /// Decode and validate an owned candidate; this never changes a live machine.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        require(
            bytes.len() >= HEADER && bytes.len() <= HEADER + MAX_PAYLOAD,
            "invalid save-state length",
        )?;
        require(&bytes[..8] == MAGIC, "invalid save-state signature")?;
        let size = u32::from_le_bytes(bytes[8..12].try_into().expect("checked header")) as usize;
        require(size == bytes.len() - HEADER, "save-state length mismatch")?;
        let payload = &bytes[HEADER..];
        require(
            Sha256::digest(payload)[..] == bytes[12..HEADER],
            "save-state checksum mismatch",
        )?;
        let saved: Saved = borsh::from_slice(payload)
            .map_err(|_| Error::Snapshot("malformed save-state payload"))?;
        Ok(Self {
            state: saved.restore()?,
        })
    }
}
