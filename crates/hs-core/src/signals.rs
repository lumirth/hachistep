use crate::time::Time;
use core::ops::ControlFlow;

/// Electrical pin drive. Board pulls and drivers resolve high impedance.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum Drive {
    Floating = 0,
    Low = 1,
    High = 2,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum Piezo {
    Negative = 0,
    Neutral = 1,
    Positive = 2,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NvDomain {
    EepromArray,
    EepromStatus,
    InternalFlash,
    Sensor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Power {
        at: Time,
        on: bool,
    },
    #[cfg(feature = "trace")]
    Bus {
        at: Time,
        pc: u16,
        address: u16,
        width: u8,
        write: bool,
        value: u16,
    },
    LcdWrite {
        at: Time,
        page: u8,
        column_byte: u16,
        value: u8,
    },
    LcdControl {
        at: Time,
        command: u8,
        parameter: Option<u8>,
    },
    Buzzer {
        at: Time,
        drive: Piezo,
    },
    Infrared {
        at: Time,
        emitting: bool,
    },
    NvByte {
        at: Time,
        domain: NvDomain,
        address: u16,
        value: u8,
    },
    NvCommit {
        at: Time,
        domain: NvDomain,
        address: u16,
        length: u16,
    },
    NvInterrupted {
        at: Time,
        domain: NvDomain,
        address: u16,
        length: u16,
    },
    Reset {
        at: Time,
        watchdog: bool,
    },
}
/// Events are delivered synchronously. NvByte reports persistent data when an
/// operation's physical progress is settled; NvCommit/NvInterrupted close its
/// enclosing address range after all affected bytes have been delivered. A
/// wrapped EEPROM write encloses the whole page. A flash pulse can
/// report progress before it ends. The callback must not re-enter the machine.
///
/// Return `Break(())` to end `Machine::run_until` after all effects at the
/// current timestamp finish. Further events at that timestamp are still delivered.
/// The returned exclusive horizon is one time quantum after those effects.
/// Immediate power operations finish completely regardless of this return value.
pub trait Output {
    fn event(&mut self, event: Event) -> ControlFlow<()>;
}
impl Output for () {
    fn event(&mut self, _: Event) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }
}
impl Output for Vec<Event> {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        self.push(event);
        ControlFlow::Continue(())
    }
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Acceleration {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
impl Acceleration {
    /// Specific force in micro-g in sensor coordinates. Includes gravity.
    pub const STILL: Self = Self {
        x: 0,
        y: 0,
        z: 1_000_000,
    };
}
#[derive(
    borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct Buttons {
    pub left: bool,
    pub center: bool,
    pub right: bool,
}
impl Buttons {
    pub const RELEASED: Self = Self {
        left: false,
        center: false,
        right: false,
    };
}
/// A package analog node driven by an electrical fixture. `None` releases
/// the override and restores the voltage supplied by the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AnalogPin {
    Pb0,
    Pb1,
    Pb2,
    Pb3,
    Pb4,
    Pb5,
    Vcref,
}
impl AnalogPin {
    pub const fn index(self) -> usize {
        self as usize
    }
}
/// Package nodes exposed for electrical fixtures.
/// A level is used only while the selected pin function is an input; release
/// restores the board pull. Output contention remains outside this fixture API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DigitalPin {
    P10,
    P11,
    P12,
    P30,
    P31,
    P32,
    P90,
    P91,
    P92,
    P93,
    Adtrg,
}
impl DigitalPin {
    pub const fn index(self) -> usize {
        self as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// Connect or remove the board's common power supply.
    Power(bool),
    Buttons(Buttons),
    Acceleration(Acceleration),
    SupplyMillivolts(u16),
    TemperatureMillicelsius(i32),
    InfraredLevel(bool),
    ResetPin(bool),
    /// Electrical fixture for the dedicated NMI pin, which idles high in user mode.
    NmiPin(bool),
    DigitalPin {
        pin: DigitalPin,
        level: Option<bool>,
    },
    AnalogPin {
        pin: AnalogPin,
        millivolts: Option<u16>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedInput {
    pub at: Time,
    pub input: Input,
}

impl Event {
    /// Exact 64.64 timestamp, independent of human-readable Debug formatting.
    pub fn time(self) -> Time {
        match self {
            Event::Power { at, .. }
            | Event::LcdWrite { at, .. }
            | Event::LcdControl { at, .. }
            | Event::Buzzer { at, .. }
            | Event::Infrared { at, .. }
            | Event::NvByte { at, .. }
            | Event::NvCommit { at, .. }
            | Event::NvInterrupted { at, .. }
            | Event::Reset { at, .. } => at,
            #[cfg(feature = "trace")]
            Event::Bus { at, .. } => at,
        }
    }
}
