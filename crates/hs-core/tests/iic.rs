//! Original physical-bus fixtures; expected bytes/periods follow section 16.
use hs_core::{
    mcu::{
        clocks::{Clocks, Frequencies},
        iic::Iic,
    },
    Time,
};

#[derive(Clone)]
struct Bus {
    iic: Iic,
    clocks: Clocks,
    now: Time,
    external: [bool; 2],
    pads: [bool; 2],
    edges: Vec<(Time, [bool; 2])>,
}
impl Bus {
    fn new() -> Self {
        let clocks = Clocks::new(
            Time::ZERO,
            Frequencies {
                main_hz: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        let mut iic = Iic::default();
        iic.set_gate(true, Time::ZERO, &clocks).unwrap();
        Self {
            iic,
            clocks,
            now: Time::ZERO,
            external: [true; 2],
            pads: [true; 2],
            edges: vec![],
        }
    }
    fn resolve(&mut self) {
        let own = self.iic.pins().unwrap_or([true; 2]);
        let pads = [own[0] && self.external[0], own[1] && self.external[1]];
        if pads != self.pads {
            self.edges.push((self.now, pads));
        }
        self.pads = pads;
        self.iic
            .input_pins(pads[0], pads[1], self.now, &self.clocks)
            .unwrap();
    }
    fn write(&mut self, reg: u16, value: u8) {
        self.iic
            .write(0xf078 + reg, value, self.now, &self.clocks)
            .unwrap();
        self.resolve();
    }
    fn read(&mut self, reg: u16) -> u8 {
        let value = self.iic.read(0xf078 + reg, self.now, &self.clocks).unwrap();
        self.resolve();
        value
    }
    fn peek(&self, reg: u16) -> u8 {
        self.iic.peek(0xf078 + reg)
    }
    fn step(&mut self) {
        let at = self
            .iic
            .deadline(&self.clocks)
            .unwrap()
            .expect("bus has scheduled work");
        assert!(at >= self.now, "past appointment");
        self.now = at;
        self.iic.advance(at, &self.clocks).unwrap();
        self.resolve();
    }
    fn run(&mut self, us: u64) {
        let end = Time::from_raw(self.now.raw() + Time::from_micros(us).raw());
        let mut visits = 0;
        while self
            .iic
            .deadline(&self.clocks)
            .unwrap()
            .is_some_and(|at| at <= end)
        {
            self.step();
            visits += 1;
            assert!(visits < 10_000, "idle loop");
        }
        self.now = end;
    }
    fn drive(&mut self, scl: bool, sda: bool) {
        self.external = [scl, sda];
        self.resolve();
        self.run(5);
    }
    fn start(&mut self) {
        self.drive(true, true);
        self.drive(true, false);
    }
    fn stop(&mut self) {
        self.drive(false, false);
        self.drive(true, false);
        self.drive(true, true);
    }
    fn slave_byte(&mut self, byte: u8) -> bool {
        for bit in (0..8).rev() {
            let high = byte & (1 << bit) != 0;
            self.drive(false, high);
            self.drive(true, high);
        }
        self.drive(false, true);
        self.drive(true, true);
        let nack = self.pads[1];
        self.drive(false, true);
        !nack
    }
    fn clock_level(&mut self, high: bool) {
        for _ in 0..100 {
            if self.pads[0] == high {
                return;
            }
            self.step();
        }
        panic!("SCL did not reach {high}");
    }
    fn master_start(&mut self, address: u8) {
        self.write(0, 0xb0);
        self.write(1, 0xbd);
        self.write(6, address);
        for _ in 0..100 {
            if self.peek(1) & 0x80 != 0 && !self.pads[0] {
                return;
            }
            self.step();
        }
        panic!("no START");
    }
    /// Fixture supplies receive data or observes transmit data, then an ACK bit.
    fn master_byte(&mut self, receive: Option<u8>, ack: bool) -> (u8, bool) {
        let mut data = 0;
        for bit in (0..8).rev() {
            self.clock_level(false);
            self.external[1] = receive.map_or(true, |v| v & (1 << bit) != 0);
            self.resolve();
            self.clock_level(true);
            data = (data << 1) | u8::from(self.pads[1]);
            self.clock_level(false);
        }
        self.external[1] = !ack;
        self.resolve();
        self.clock_level(true);
        let nack = self.pads[1];
        self.clock_level(false);
        self.external[1] = true;
        self.resolve();
        self.run(3);
        (data, nack)
    }
}

#[test]
fn reset_map_holding_order_and_read_qualified_flags() {
    let mut b = Bus::new();
    assert_eq!(
        (0..8).map(|r| b.read(r)).collect::<Vec<_>>(),
        [0, 0x7d, 0x38, 0, 0, 0, 0xff, 0xff]
    );
    b.write(2, 0x88);
    b.write(6, 1);
    assert_eq!(b.read(6), 0x80);
    b.write(0, 0x90);
    assert_eq!(b.peek(4), 0x80);
    b.write(4, 0);
    assert_eq!(b.peek(4), 0x80);
    b.read(4);
    b.write(4, 0);
    assert_eq!(b.peek(4), 0);
}

#[test]
fn slave_address_general_call_and_no_match_use_the_same_shifter() {
    for (address, flags, transmit, ack) in [
        (0x54, 0x22, false, true),
        (0x55, 0x82, true, true),
        (0, 0x23, false, true),
        (0x56, 0, false, false),
    ] {
        let mut b = Bus::new();
        b.write(5, 0x54);
        b.write(0, 0x80);
        b.start();
        assert_eq!(b.slave_byte(address), ack, "address {address:02x}");
        assert_eq!(b.peek(4), flags);
        assert_eq!(b.peek(0) & 0x10 != 0, transmit);
        if flags & 0x20 != 0 {
            assert_eq!(b.read(7), address);
        }
        if !transmit {
            b.stop();
            assert_eq!(b.peek(1) & 0x80, 0);
            assert_eq!(b.peek(4) & 8, if ack { 8 } else { 0 });
        }
    }
}

#[test]
fn master_transmit_ack_nack_stop_and_clock_period() {
    for (cks, period) in [(0, 28), (5, 100)] {
        let mut b = Bus::new();
        b.write(0, 0x80 | cks);
        b.master_start(0xa0);
        b.write(0, 0xb0 | cks);
        assert_eq!(b.master_byte(None, true), (0xa0, false));
        assert_eq!(b.peek(4) & 0xc0, 0xc0);
        assert_eq!(b.peek(3) & 2, 0);
        let rises: Vec<_> = b
            .edges
            .windows(2)
            .filter_map(|p| (!p[0].1[0] && p[1].1[0]).then_some(p[1].0.as_micros()))
            .collect();
        assert_eq!(rises[rises.len() - 1] - rises[rises.len() - 2], period);
        b.write(1, 0x3d);
        b.run(500);
        assert_eq!(b.peek(1) & 0x80, 0);
        assert_eq!(b.peek(4) & 8, 8);
        assert_eq!(b.peek(0) & 0x30, 0x30);
    }
    let mut b = Bus::new();
    b.master_start(0xa0);
    b.write(3, 4);
    assert_eq!(b.master_byte(None, false), (0xa0, true));
    assert_eq!(b.peek(4) & 0xd0, 0xd0);
    assert_eq!(b.peek(3) & 2, 2);
}

#[test]
fn master_single_receive_stops_without_a_second_dummy_start() {
    let mut b = Bus::new();
    b.master_start(0xa1);
    b.master_byte(None, true);
    b.read(4);
    b.write(4, 0);
    b.write(0, 0xe0);
    b.write(3, 1);
    b.read(7);
    assert_eq!(b.master_byte(Some(0xa5), false), (0xa5, true));
    assert_eq!(b.peek(4) & 0x20, 0x20);
    assert_eq!(b.read(7), 0xa5);
    let edges = b.edges.len();
    b.run(1000);
    assert_eq!(b.edges.len(), edges);
    assert!(!b.pads[0]);
    b.write(1, 0x3d);
    b.run(100);
    assert_eq!(b.peek(4) & 8, 8);
}

#[test]
fn slave_receive_holds_before_ack_until_previous_data_is_read() {
    let mut b = Bus::new();
    b.write(5, 0x54);
    b.write(0, 0x80);
    b.start();
    b.slave_byte(0x54);
    b.read(7);
    assert!(b.slave_byte(0x3c));
    for bit in (0..8).rev() {
        let high = 0xa5 & (1 << bit) != 0;
        b.drive(false, high);
        b.drive(true, high);
    }
    b.drive(false, true);
    b.drive(true, true);
    assert!(!b.pads[0]);
    assert_eq!(b.peek(7), 0x3c);
    assert_eq!(b.read(7), 0x3c);
    b.run(12);
    assert!(b.pads[0]);
    assert_eq!(b.read(7), 0xa5);
}

#[test]
fn filter_rejects_one_sample_and_reset_keeps_start_stop_detection() {
    let mut b = Bus::new();
    b.write(0, 0x80);
    b.external[1] = false;
    b.resolve();
    b.run(1);
    b.external[1] = true;
    b.resolve();
    b.run(3);
    assert_eq!(b.peek(1) & 0x80, 0);
    b.start();
    assert_ne!(b.peek(1) & 0x80, 0);
    b.write(1, 0x7f);
    assert_eq!(b.iic.pins(), Some([true, true]));
    b.stop();
    assert_eq!(b.peek(1) & 0x80, 0);
}

#[test]
fn arbitration_and_external_clock_stretch_observe_resolved_pads() {
    let mut b = Bus::new();
    b.master_start(0xa0);
    b.external[0] = false;
    b.resolve();
    b.run(100);
    assert!(!b.pads[0]);
    assert_eq!(b.peek(4) & 4, 0);
    b.external[1] = false;
    b.external[0] = true;
    b.resolve();
    b.run(5);
    assert_eq!(b.peek(4) & 4, 4);
    assert_eq!(b.peek(0) & 0x30, 0);
}

#[test]
fn synchronous_receive_overrun_retains_the_first_byte_and_requests_nak_irq() {
    let mut b = Bus::new();
    b.write(5, 1);
    b.write(3, 0x10);
    b.write(0, 0xa0);
    for byte in [0x3c, 0xc3] {
        for bit in (0..8).rev() {
            b.clock_level(false);
            b.external[1] = byte & (1 << bit) != 0;
            b.resolve();
            b.clock_level(true);
            b.run(3); // Let both pad-filter latches observe this rise.
            if bit != 0 || byte == 0x3c {
                b.clock_level(false);
            }
        }
    }
    assert_eq!(b.read(7), 0x3c);
    assert_eq!(b.peek(4) & 4, 4);
    assert_eq!(b.peek(0) & 0x20, 0);
    assert!(b.iic.interrupt());
}

#[test]
fn three_bit_data_follows_a_full_eight_bit_address() {
    let mut b = Bus::new();
    b.write(2, 3);
    b.master_start(0xa0);
    assert_eq!(b.master_byte(None, true).0, 0xa0);
    b.write(2, 3);
    b.write(6, 0xa0);
    let mut bits = 0;
    for _ in 0..3 {
        b.clock_level(true);
        bits = bits * 2 + u8::from(b.pads[1]);
        b.clock_level(false);
    }
    b.external[1] = false;
    b.resolve();
    b.clock_level(true);
    b.clock_level(false);
    b.run(3);
    assert_eq!(bits, 5);
    assert_eq!(b.peek(2) & 7, 0);
    assert_eq!(b.peek(4) & 0xc0, 0xc0);
}

#[test]
fn wait_insertion_extends_only_the_pre_ack_low_by_two_periods() {
    let mut b = Bus::new();
    b.write(2, 0x48);
    b.master_start(0xa0);
    b.master_byte(None, true);
    let rises: Vec<_> = b
        .edges
        .windows(2)
        .filter_map(|p| (!p[0].1[0] && p[1].1[0]).then_some(p[1].0.as_micros()))
        .collect();
    assert_eq!(rises[rises.len() - 1] - rises[rises.len() - 2], 84);
    assert_eq!(rises[rises.len() - 2] - rises[rises.len() - 3], 28);
}

#[test]
fn a_gated_half_phi_monitor_keeps_its_remaining_clock_work() {
    let mut b = Bus::new();
    b.master_start(0xa0);
    b.external[0] = false;
    b.resolve();
    // SCL is released after 14 phi; its first monitor follows 7.5 phi later.
    while !b.iic.pins().unwrap()[0] {
        b.step();
    }
    b.run(5);
    let before = b.clone();
    b.iic.set_gate(false, b.now, &b.clocks).unwrap();
    b.run(100);
    b.iic.set_gate(true, b.now, &b.clocks).unwrap();
    let mut a = before;
    a.run(2);
    b.run(2);
    let da = a.iic.deadline(&a.clocks).unwrap().unwrap().raw() - a.now.raw();
    let db = b.iic.deadline(&b.clocks).unwrap().unwrap().raw() - b.now.raw();
    assert_eq!(da, db);
    // Rational edge rounding differs from truncating a duration by one 64.64 quantum.
    assert!(da.abs_diff(Time::from_micros(1).raw() / 2) <= 1);
    a.external = [true, true];
    b.external = [true, true];
    a.resolve();
    b.resolve();
    a.run(100);
    b.run(100);
    assert_eq!(a.peek(2), b.peek(2));
    assert_eq!(a.pads, b.pads);
}
