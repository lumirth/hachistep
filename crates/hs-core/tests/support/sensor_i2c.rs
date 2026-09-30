//! External open-drain master for persistence/allocation checks. Hardware-value
//! assertions belong to hachiware's separately authored GPIO guests.
use hs_core::{DigitalPin, Images, Input, Machine, Time, TimedInput};

struct Bus {
    us: u64,
    inputs: Vec<TimedInput>,
}

impl Bus {
    fn pin(&mut self, pin: DigitalPin, level: Option<bool>) {
        self.inputs.push(TimedInput {
            at: Time::from_micros(self.us),
            input: Input::DigitalPin { pin, level },
        });
        self.us += 2;
    }
    fn clock(&mut self, high: bool) {
        self.pin(DigitalPin::P91, Some(high));
    }
    fn data(&mut self, high: bool) {
        self.pin(DigitalPin::P92, if high { None } else { Some(false) });
    }
    fn start(&mut self) {
        self.clock(false);
        self.data(true);
        self.clock(true);
        self.data(false);
        self.clock(false);
    }
    fn stop(&mut self) {
        self.data(false);
        self.clock(true);
        self.data(true);
    }
    fn send(&mut self, value: u8) {
        for bit in 0..8 {
            self.data(value & (0x80 >> bit) != 0);
            self.clock(true);
            self.clock(false);
        }
        self.data(true);
        self.clock(true);
        self.clock(false);
    }
    fn read(&mut self, ack: bool) {
        self.data(true);
        for _ in 0..8 {
            self.clock(true);
            self.clock(false);
        }
        self.data(!ack);
        self.clock(true);
        self.clock(false);
        self.data(true);
    }
}

pub fn session() -> (Machine, Vec<TimedInput>, Time) {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&[1, 0]);
    // Enable the internal SDA pull-up while keeping all P9 drivers released.
    rom[0x100..0x108].copy_from_slice(&[0xf8, 4, 0x6a, 0x88, 0xf0, 0x87, 0x40, 0xfe]);
    let m = Machine::new(Images {
        firmware: &rom,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap();
    let mut bus = Bus {
        us: 4000,
        inputs: Vec::new(),
    };
    bus.start();
    bus.send(0x70);
    bus.send(2);
    bus.start();
    bus.send(0x71);
    bus.read(true);
    bus.read(false);
    bus.stop();
    // Include sleep, wake and reset, notably the accepted command's last ACK.
    for command in [1, 0, 2] {
        bus.start();
        for byte in [0x70, 0x0a, command] {
            bus.send(byte);
        }
        bus.stop();
        bus.us += 50;
    }
    bus.start();
    bus.send(0x70);
    bus.send(0);
    bus.stop();
    bus.start();
    bus.send(0x71);
    bus.read(false);
    bus.stop();
    (m, bus.inputs, Time::from_micros(bus.us + 100))
}
