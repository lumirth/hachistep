//! Unsafe code is restricted to this allocator-observation test harness.
//! The production library forbids unsafe code and uses safe library interfaces.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[path = "support/flash.rs"]
mod flash_guest;
#[path = "support/sensor_i2c.rs"]
mod sensor_i2c;
struct Count;
static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static GLOBAL: Count = Count;
#[test]
fn ordinary_run_has_no_heap_allocation() {
    let mut firmware = vec![0; 49152];
    firmware[0..2].copy_from_slice(&[1, 0]);
    firmware[0x100..0x102].copy_from_slice(&[0x40, 0xfe]);
    let mut m = hs_core::Machine::new(hs_core::Images {
        firmware: &firmware,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
    })
    .unwrap();
    let motion = [(3200, 1_000_000), (3250, 0)].map(|(us, x)| hs_core::TimedInput {
        at: hs_core::Time::from_micros(us),
        input: hs_core::Input::Acceleration(hs_core::Acceleration {
            x,
            y: 0,
            z: 1_000_000,
        }),
    });
    ENABLED.store(true, Ordering::SeqCst);
    let result = m.run_until(hs_core::Time::from_micros(10000), &motion, &mut ());
    ENABLED.store(false, Ordering::SeqCst);
    result.unwrap();
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);

    ENABLED.store(true, Ordering::SeqCst);
    let result = (|| -> Result<(), hs_core::Error> {
        m.power_off(&mut ())?;
        m.run_until(hs_core::Time::from_micros(30000), &[], &mut ())?;
        m.power_on(&mut ())?;
        m.run_until(hs_core::Time::from_micros(50000), &[], &mut ())?;
        Ok(())
    })();
    ENABLED.store(false, Ordering::SeqCst);
    result.unwrap();
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);

    // Construction and snapshots may allocate; executing custom programming
    // firmware after either operation must retain the same execution contract.
    let mut m = flash_guest::machine();
    let encoded = m.snapshot().encode().unwrap();
    m = hs_core::Machine::from_snapshot(&hs_core::Snapshot::decode(&encoded).unwrap());
    ENABLED.store(true, Ordering::SeqCst);
    let result = m.run_until(hs_core::Time::from_micros(12000), &[], &mut ());
    ENABLED.store(false, Ordering::SeqCst);
    result.unwrap();
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(m.firmware()[0x9000], 0);

    let (mut sensor, bus, end) = sensor_i2c::session();
    ENABLED.store(true, Ordering::SeqCst);
    let result = sensor.run_until(end, &bus, &mut ());
    ENABLED.store(false, Ordering::SeqCst);
    result.unwrap();
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);

    let mut boot = hs_core::Machine::new(hs_core::Images {
        firmware: &[0; 49152],
        eeprom: &[255; 65536],
        eeprom_status: 0,
    })
    .unwrap();
    use hs_core::{DigitalPin, Input, Time, TimedInput};
    let mut rows = vec![
        TimedInput {
            at: Time::ZERO,
            input: Input::ResetPin(false),
        },
        TimedInput {
            at: Time::ZERO,
            input: Input::NmiPin(false),
        },
        TimedInput {
            at: Time::from_micros(100),
            input: Input::ResetPin(true),
        },
    ];
    for (start, byte) in [(1000, 0u8), (20_000, 0x55)] {
        for bit in 0..10 {
            rows.push(TimedInput {
                at: Time::from_micros(start + bit * 1250 / 3),
                input: Input::DigitalPin {
                    pin: DigitalPin::P31,
                    level: Some(bit == 9 || (bit != 0 && byte & (1 << (bit - 1)) != 0)),
                },
            });
        }
    }
    ENABLED.store(true, Ordering::SeqCst);
    let result = boot.run_until(Time::from_micros(1_000_000), &rows, &mut ());
    ENABLED.store(false, Ordering::SeqCst);
    result.unwrap();
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);
    assert!(boot.firmware().iter().all(|v| *v == 255));
}
