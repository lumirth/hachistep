//! CSB-high SDI/SCK protocol. Register effects and read shadows are shared with
//! SPI. See docs/accuracy/bma150-i2c.md for edge placement and pointer retention.
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
    pub(super) fn effect_edges(&self) -> u8 {
        match self.phase {
            Phase::Idle => 15,
            Phase::Receive(_) => (8 - self.bits) * 2 - 1,
            _ => 1,
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
    /// Observe the previous settled bus and current pins before slave effects.
    /// The board stores the newly settled pins after applying our SDA intent.
    pub(crate) fn i2c_pins(
        &mut self,
        previous: [bool; 2],
        current: [bool; 2],
        now: Time,
    ) -> Result<(), Error> {
        if self.selected || self.unpowered_since.is_some() || self.serial_ready.is_some() {
            self.i2c.abort();
            return Ok(());
        }
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
                                _ => return Ok(()), // No address ACK; wait for START.
                            },
                            Receive::Control => {
                                self.i2c.pointer = byte & 0x7f;
                                Following::Data
                            }
                            Receive::Data => {
                                self.write_register(self.i2c.pointer, byte, now)?;
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
        Ok(())
    }

    fn i2c_read(&mut self) {
        self.tx = self.prepare_read(self.i2c.pointer);
        self.i2c.low = self.tx.unwrap_or(0xff) & 0x80 == 0;
        self.i2c.phase = Phase::Read { sampled: 0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bus {
        sensor: Bma150,
        pins: [bool; 2],
        now: Time,
    }
    impl Bus {
        fn new() -> Self {
            let mut sensor = Bma150::new(Time::ZERO);
            let now = Time::from_micros(4000);
            while let Some(at) = sensor.deadline().filter(|at| *at <= now) {
                sensor.at_deadline(at, &mut ()).unwrap();
            }
            Self {
                sensor,
                pins: [true, true],
                now,
            }
        }
        fn edge(&mut self, clock: bool, release: bool) -> bool {
            self.now = self.now.checked_add(Duration::from_micros(1)).unwrap();
            while let Some(at) = self.sensor.deadline().filter(|at| *at <= self.now) {
                self.sensor.at_deadline(at, &mut ()).unwrap();
            }
            let data = release && self.sensor.data_output() != Drive::Low;
            self.sensor
                .i2c_pins(self.pins, [clock, data], self.now)
                .unwrap();
            self.pins = [clock, release && self.sensor.data_output() != Drive::Low];
            self.pins[1]
        }
        fn start(&mut self) {
            self.edge(false, true);
            self.edge(true, true);
            self.edge(true, false);
            self.edge(false, false);
        }
        fn send(&mut self, byte: u8) {
            for bit in 0..8 {
                let high = byte & (0x80 >> bit) != 0;
                self.edge(false, high);
                self.edge(true, high);
                self.edge(false, high);
            }
            self.edge(false, true);
            assert!(!self.edge(true, true), "slave ACK");
            self.edge(false, true);
        }
        fn pointer(&mut self, address: u8) {
            self.start();
            self.send(0x70);
            self.send(address);
            self.start();
            self.send(0x71);
        }
    }

    #[test]
    fn data_sampling_acknowledges_freshness_even_when_the_master_nacks() {
        let mut bus = Bus::new();
        bus.pointer(2);
        assert_eq!(bus.sensor.peek(2).unwrap() & 1, 1, "only prepared");
        let mut value = 0;
        for bit in 0..8 {
            value = value << 1 | u8::from(bus.edge(true, true));
            assert_eq!(bus.sensor.peek(2).unwrap() & 1, 0, "data bit {bit}");
            bus.edge(false, true);
        }
        assert_eq!(value, 1, "the launched byte retains its old fresh flag");
        bus.edge(true, true); // Master NACK.
        bus.edge(false, true);
        assert_eq!(bus.sensor.peek(2).unwrap() & 1, 0);
        for _ in 0..9 {
            assert!(bus.edge(true, true));
            assert!(bus.edge(false, true));
        }
    }

    #[test]
    fn reset_ack_completes_inside_the_serial_quiet_interval() {
        let mut bus = Bus::new();
        bus.start();
        for byte in [0x70, 0x0a, 2] {
            bus.send(byte); // Includes an asserted ACK at the ninth rise.
        }
        assert!(bus.sensor.quiet_deadline.unwrap() > bus.now);
        assert_eq!(bus.sensor.data_output(), Drive::Floating);
        bus.sensor.validate(bus.now).unwrap();
    }

    #[test]
    fn retained_power_loss_aborts_output_and_requires_a_new_start() {
        let mut bus = Bus::new();
        bus.pointer(0); // Chip ID starts with a low bit.
        assert_eq!(bus.sensor.data_output(), Drive::Low);
        bus.sensor.set_supply(2000, bus.now, &mut ()).unwrap();
        assert_eq!(bus.sensor.data_output(), Drive::Floating);
        bus.now = bus.now.checked_add(Duration::from_micros(1000)).unwrap();
        bus.sensor.set_supply(3000, bus.now, &mut ()).unwrap();
        for _ in 0..9 {
            assert!(bus.edge(true, true));
            assert!(bus.edge(false, true));
        }
        bus.pointer(0);
        let mut byte = 0;
        for _ in 0..8 {
            byte = byte << 1 | u8::from(bus.edge(true, true));
            bus.edge(false, true);
        }
        assert_eq!(byte, 2);
        bus.sensor.validate(bus.now).unwrap();
    }
}
