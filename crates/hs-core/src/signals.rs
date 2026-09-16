use crate::time::Time;

/// A pin is not a byte. High impedance is resolved by board pulls/drivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    Floating,
    Low,
    High,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piezo {
    Negative,
    Neutral,
    Positive,
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
    Reset {
        at: Time,
        watchdog: bool,
    },
}
/// Events are delivered synchronously. NvByte records precede their NvCommit
/// at one timestamp, so consumers never have to infer intermediate writes from
/// a later memory image. No re-entry is allowed.
pub trait Output {
    fn event(&mut self, event: Event);
}
impl Output for () {
    fn event(&mut self, _: Event) {}
}
impl Output for Vec<Event> {
    fn event(&mut self, event: Event) {
        self.push(event);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
/// A real package analog node. Explicit voltages are electrical-fixture
/// stimuli, not extra controls wired into an unmodified product. `None` releases
/// the override and restores the ordinary board-derived voltage.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Buttons(Buttons),
    Acceleration(Acceleration),
    SupplyMillivolts(u16),
    InfraredLevel(bool),
    ResetPin(bool),
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
            | Event::Reset { at, .. } => at,
            #[cfg(feature = "trace")]
            Event::Bus { at, .. } => at,
        }
    }
}
