//! Unsafe code is restricted to this allocator-observation test harness.
//! The production library forbids unsafe code and has no external dependencies.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[path = "support/flash.rs"]
mod flash_guest;
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
    ENABLED.store(true, Ordering::SeqCst);
    let result = m.run_until(hs_core::Time::from_micros(10000), &[], &mut ());
    ENABLED.store(false, Ordering::SeqCst);
    result.unwrap();
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);

    // Construction and snapshots may allocate; executing custom programming
    // firmware after either operation must retain the same execution contract.
    let mut m = flash_guest::machine();
    m = hs_core::Machine::from_snapshot(&m.snapshot());
    ENABLED.store(true, Ordering::SeqCst);
    let result = m.run_until(hs_core::Time::from_micros(12000), &[], &mut ());
    ENABLED.store(false, Ordering::SeqCst);
    result.unwrap();
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(m.firmware()[0x9000], 0);
}
