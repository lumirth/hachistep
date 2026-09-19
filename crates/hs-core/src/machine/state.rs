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
        let mut mcu = m.mcu.clone();
        require(
            !mcu.sync(m.observation_time(), &mut ())?,
            "unprocessed reset at capture",
        )?;
        Ok(Self {
            firmware_origin: m.firmware_origin,
            now: m.now,
            last_effect: m.last_effect,
            cpu: m.cpu.save()?,
            mcu,
            eeprom: m.eeprom.clone(),
            sensor: m.sensor.clone(),
            lcd: m.lcd.clone(),
            conditions: m.conditions,
            analog_pins: m.analog_pins,
            pending: m.pending.map(|mut p| {
                if p.lane == 0 {
                    p.high = 0;
                }
                p
            }),
            resume_after: m.resume_after,
            serial: m.serial,
            piezo: m.piezo,
            incident_light: m.incident_light,
            emitting: m.emitting,
            reset_asserted: m.reset_asserted,
            reset_release: m.reset_release,
            watchdog_reset: m.watchdog_reset,
            connected: m.connected,
            power: m.power.clone(),
            stopped_by_core_fault: m.fault.is_some(),
        })
    }
    fn restore(self) -> Result<Machine, Error> {
        let mut m = Machine {
            firmware_origin: self.firmware_origin,
            now: self.now,
            last_effect: self.last_effect,
            cpu: self.cpu.restore(self.stopped_by_core_fault)?,
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
            changed_peripherals: 0,
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
            let action = m.cpu.issued_action();
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
                resume.wait().uses(tap) && m.cpu.sleeping() && m.pending.is_none(),
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
        Ok(m)
    }
}
impl Snapshot {
    /// Maximum native file size, for bounded frontend reads.
    pub const MAX_ENCODED_SIZE: usize = HEADER + MAX_PAYLOAD;

    /// Immutable identity of the construction firmware, before guest flash writes.
    pub fn firmware_origin(&self) -> [u8; 32] {
        self.state.firmware_origin
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

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        let mut rom = vec![0; 49152];
        rom[..2].copy_from_slice(&[1, 0]);
        rom[0x100..0x102].copy_from_slice(&[0x40, 0xfe]);
        Machine::new(Images {
            firmware: &rom,
            eeprom: &[0xff; 65536],
            eeprom_status: 0,
        })
        .unwrap()
    }
    fn encoded(saved: &Saved) -> Vec<u8> {
        let payload = borsh::to_vec(saved).unwrap();
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&Sha256::digest(&payload));
        bytes.extend(payload);
        bytes
    }
    #[test]
    fn checksum_does_not_substitute_for_progress_validation() {
        let mut m = machine();
        m.run_until(Time::from_micros(10), &[], &mut ()).unwrap();
        let before = m.snapshot();
        for change in 0..4 {
            let mut saved = Saved::capture(&m).unwrap();
            match change {
                0 => saved.last_effect = Time::MAX,
                1 => saved.mcu.clocks.frequencies.main_hz = 0,
                2 => saved.pending.as_mut().unwrap().lane = 2,
                _ => saved.conditions.avcc_override_millivolts = Some(0),
            }
            assert!(Snapshot::decode(&encoded(&saved)).is_err());
            assert_eq!(m.snapshot(), before);
        }
    }
    #[test]
    fn corrupted_scalar_records_are_bounded_and_never_panic() {
        let m = machine();
        let saved = Saved::capture(&m).unwrap();
        let bytes = encoded(&saved);
        // Cover CPU, clock/control, serial, sensor and board scalar regions;
        // random mutations of ROM/RAM bytes alone would barely exercise loading.
        let cpu_end = HEADER + 64 + borsh::to_vec(&saved.cpu).unwrap().len();
        let flash_end = cpu_end + borsh::to_vec(&saved.mcu.flash).unwrap().len();
        let mcu_end = cpu_end + borsh::to_vec(&saved.mcu).unwrap().len();
        let eeprom_end = mcu_end + borsh::to_vec(&saved.eeprom).unwrap().len();
        let ranges = [
            HEADER + 64..cpu_end,
            flash_end + 2048..mcu_end,
            mcu_end + 65536..eeprom_end,
            eeprom_end..eeprom_end + borsh::to_vec(&saved.sensor).unwrap().len(),
            bytes.len() - 256..bytes.len(),
        ];
        for at in ranges.into_iter().flat_map(|r| r.step_by(17)) {
            for value in [0, 0xff] {
                let mut bad = bytes.clone();
                bad[at..(at + 8).min(bytes.len())].fill(value);
                let checksum = Sha256::digest(&bad[HEADER..]);
                bad[12..HEADER].copy_from_slice(&checksum);
                if let Ok(snapshot) = Snapshot::decode(&bad) {
                    let mut candidate = Machine::from_snapshot(&snapshot);
                    let _ = candidate.run_until(Time::from_micros(100), &[], &mut ());
                    let _ = candidate.snapshot().encode().unwrap();
                }
            }
        }
    }
}
