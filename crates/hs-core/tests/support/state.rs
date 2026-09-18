use hs_core::{Machine, Snapshot};

pub fn assert_same_state(a: &Machine, b: &Machine) {
    let a = a.snapshot().encode().unwrap();
    let b = b.snapshot().encode().unwrap();
    assert!(
        a == b,
        "causal state differs at byte {:?} (lengths {} / {})",
        a.iter().zip(&b).position(|(a, b)| a != b),
        a.len(),
        b.len()
    );
}
pub fn restore_file(snapshot: &Snapshot) -> Machine {
    let bytes = snapshot.encode().unwrap();
    let loaded = Snapshot::decode(&bytes).unwrap();
    Machine::from_snapshot(&loaded)
}
