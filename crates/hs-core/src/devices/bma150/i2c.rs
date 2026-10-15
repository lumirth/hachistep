//! CSB-high SDI/SCK protocol. Register effects and read shadows are shared with
//! SPI. The BMA150 datasheet section 4.1 defines the serial edge sequences.
use super::*;

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Receive {
    Address = 0,
    Control = 1,
    Data = 2,
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Following {
    Control = 0,
    Data = 1,
    Read = 2,
    Idle = 3,
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Phase {
    Idle = 0,
    Receive(Receive) = 1,
    // 0: eighth rise passed; 1: ACK driven; 2: ninth rise passed.
    Acknowledge { following: Following, edge: u8 } = 2,
    Read { sampled: u8 } = 3,
    MasterAcknowledge(Option<bool>) = 4,
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct State {
    phase: Phase,
    pointer: u8,
    bits: u8,
    shift: u8,
    pub(super) low: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            pointer: 0,
            bits: 0,
            shift: 0,
            low: false,
        }
    }
}

impl State {
    /// Shift clock edges before one can change the parser. Every shift edge
    /// moves SCL, so an idle parser, which waits for SDA to fall while SCL
    /// stays high, sees no START from a frame's edges.
    pub(super) fn effect_edges(&self) -> u8 {
        match self.phase {
            Phase::Idle => 16,
            Phase::Receive(_) => (8 - self.bits) * 2 - 1,
            _ => 1,
        }
    }

    /// Waiting for START with nothing retained but the register pointer.
    pub(super) fn idle(&self) -> bool {
        *self
            == Self {
                pointer: self.pointer,
                ..Self::default()
            }
    }

    pub(super) fn abort(&mut self) {
        *self = Self {
            pointer: self.pointer,
            ..Self::default()
        };
    }

    pub(super) fn validate(&self, available: bool) -> Result<(), Error> {
        use crate::state::require;
        require(
            self.pointer < 128
                && (available || self.phase == Phase::Idle)
                && match self.phase {
                    Phase::Receive(_) => {
                        !self.low && self.bits < 8 && u16::from(self.shift) < (1 << self.bits)
                    }
                    phase => {
                        self.bits == 0
                            && self.shift == 0
                            && match phase {
                                Phase::Idle | Phase::MasterAcknowledge(_) => !self.low,
                                Phase::Acknowledge { edge, .. } => {
                                    edge <= 2 && self.low == (edge != 0)
                                }
                                Phase::Read { sampled } => sampled <= 8,
                                Phase::Receive(_) => unreachable!(),
                            }
                    }
                },
            "invalid sensor I2C state",
        )
    }
}

impl Bma150 {
    /// The board materializes converted state at writes, launches and read
    /// acknowledgement. Quiet I2C shifts retain their parser-only behavior.
    pub(crate) fn i2c_pins_at(
        &mut self,
        previous: [bool; 2],
        current: [bool; 2],
        now: Time,
    ) -> Result<bool, Error> {
        if !self.selected && self.unpowered_since.is_none() && self.serial_ready.is_none() {
            let [old_clock, _] = previous;
            let [clock, _] = current;
            let consumes_conversion = if !old_clock && clock {
                matches!(self.i2c.phase, Phase::Read { sampled: 0 })
                    || matches!(self.i2c.phase, Phase::Receive(Receive::Data)) && self.i2c.bits == 7
            } else if old_clock && !clock {
                matches!(
                    self.i2c.phase,
                    Phase::Acknowledge {
                        following: Following::Read,
                        edge: 2
                    } | Phase::MasterAcknowledge(Some(true))
                )
            } else {
                false
            };
            if consumes_conversion && self.quiet_deadline.is_none() {
                self.sync_local_until(now, true)?;
            }
        }
        self.i2c_pins(previous, current, now)
    }
    /// Observe the previous settled bus and current pins before slave effects.
    /// The board stores the newly settled pins after applying our SDA intent.
    pub(crate) fn i2c_pins(
        &mut self,
        previous: [bool; 2],
        current: [bool; 2],
        now: Time,
    ) -> Result<bool, Error> {
        if self.selected || self.unpowered_since.is_some() || self.serial_ready.is_some() {
            self.i2c.abort();
            return Ok(false);
        }
        let mut deadline_changed = false;
        let [old_clock, old_data] = previous;
        let [clock, data] = current;
        if old_clock && clock && old_data != data {
            self.i2c.abort();
            self.tx = None;
            self.tx_pair = None;
            self.tx_shadow = false;
            if !data && self.quiet_deadline.is_none() {
                self.i2c.phase = Phase::Receive(Receive::Address);
            }
        } else if self.quiet_deadline.is_some()
            && !matches!(self.i2c.phase, Phase::Acknowledge { .. })
        {
            self.i2c.abort();
        } else if !old_clock && clock {
            match self.i2c.phase {
                Phase::Receive(kind) => {
                    self.i2c.shift = self.i2c.shift << 1 | u8::from(data);
                    self.i2c.bits += 1;
                    if self.i2c.bits == 8 {
                        let byte = self.i2c.shift;
                        self.i2c.abort();
                        let following = match kind {
                            Receive::Address => match byte {
                                0x70 => Following::Control,
                                0x71 => Following::Read,
                                _ => return Ok(false), // No address ACK; wait for START.
                            },
                            Receive::Control => {
                                self.i2c.pointer = byte & 0x7f;
                                Following::Data
                            }
                            Receive::Data => {
                                self.write_register(self.i2c.pointer, byte, now)?;
                                deadline_changed = true;
                                if self.quiet_deadline.is_some() {
                                    Following::Idle // Finish the accepted reset's ACK.
                                } else {
                                    Following::Control
                                }
                            }
                        };
                        self.i2c.phase = Phase::Acknowledge { following, edge: 0 };
                    }
                }
                Phase::Acknowledge { following, .. } => {
                    self.i2c.phase = Phase::Acknowledge { following, edge: 2 };
                }
                Phase::Read { sampled } => {
                    if sampled == 0 {
                        self.acknowledge_read(self.i2c.pointer);
                    }
                    if sampled < 8 {
                        self.i2c.phase = Phase::Read {
                            sampled: sampled + 1,
                        };
                        if sampled == 7 {
                            self.i2c.pointer = self.i2c.pointer.wrapping_add(1) & 0x7f;
                        }
                    }
                }
                Phase::MasterAcknowledge(_) => {
                    self.i2c.phase = Phase::MasterAcknowledge(Some(!data));
                }
                Phase::Idle => {}
            }
        } else if old_clock && !clock {
            match self.i2c.phase {
                Phase::Acknowledge { following, edge: 0 } => {
                    self.i2c.low = true;
                    self.i2c.phase = Phase::Acknowledge { following, edge: 1 };
                }
                Phase::Acknowledge { following, edge: 2 } => {
                    self.i2c.low = false;
                    self.i2c.phase = match following {
                        Following::Control => Phase::Receive(Receive::Control),
                        Following::Data => Phase::Receive(Receive::Data),
                        Following::Read => {
                            self.i2c_read();
                            self.i2c.phase
                        }
                        Following::Idle => Phase::Idle,
                    };
                }
                Phase::Read { sampled } => {
                    if sampled == 8 {
                        self.i2c.low = false;
                        self.i2c.phase = Phase::MasterAcknowledge(None);
                    } else {
                        self.i2c.low = self.tx.unwrap_or(0xff) & (0x80 >> sampled) == 0;
                    }
                }
                Phase::MasterAcknowledge(Some(true)) => self.i2c_read(),
                Phase::MasterAcknowledge(Some(false)) => self.i2c.abort(),
                _ => {}
            }
        }
        Ok(deadline_changed)
    }

    fn i2c_read(&mut self) {
        self.tx = self.prepare_read(self.i2c.pointer);
        self.i2c.low = self.tx.unwrap_or(0xff) & 0x80 == 0;
        self.i2c.phase = Phase::Read { sampled: 0 };
    }
}
