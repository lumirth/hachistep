#[path = "support/sensor_i2c.rs"]
mod sensor_i2c;
#[path = "support/state.rs"]
mod state;
use hs_core::Time;

#[test]
fn native_replay_at_each_bus_edge_preserves_partial_bytes_and_both_ack_halves() {
    let (mut whole, inputs, end) = sensor_i2c::session();
    let mut split = whole.clone();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    whole.run_until(end, &inputs, &mut a).unwrap();
    let mut consumed = 0;
    for input in &inputs {
        // A horizon is exclusive. One representable instant later captures
        // the pin consequence, before another CPU or device clock edge.
        let at = Time::from_raw(input.at.raw() + 1);
        consumed += split
            .run_until(at, &inputs[consumed..], &mut b)
            .unwrap()
            .inputs_consumed;
        split = state::restore_file(&split.snapshot());
    }
    split.run_until(end, &inputs[consumed..], &mut b).unwrap();
    assert_eq!(a, b);
    state::assert_same_state(&whole, &split);
}
