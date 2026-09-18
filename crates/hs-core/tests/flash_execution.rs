#[path = "support/flash.rs"]
mod guest;
#[path = "support/state.rs"]
mod state;
use hs_core::{Input, Machine, Time, TimedInput};

#[test]
fn ram_programming_and_verify_survive_partition_and_mid_pulse_restoration() {
    let mut whole = guest::machine();
    let mut split = whole.clone();
    let (mut a, mut b) = (vec![], vec![]);
    whole
        .run_until(Time::from_micros(12000), &[], &mut a)
        .unwrap();
    let mut us = 0;
    while us < 12000 {
        us = (us + 137).min(12000);
        split.run_until(Time::from_micros(us), &[], &mut b).unwrap();
        let before = split.snapshot();
        let _ = split.firmware();
        let _ = split.peek(0x9000).unwrap();
        assert_eq!(
            before,
            split.snapshot(),
            "projection cannot complete or protect a pulse"
        );
        split = state::restore_file(&before);
    }
    assert_eq!(whole.firmware()[0x9000], 0);
    assert_ne!(&whole.ram()[0x180..0x182], &[0xff, 0xff]);
    state::assert_same_state(&whole, &split);
    assert_eq!(a, b);
}

#[test]
fn power_loss_and_mcu_reset_retain_partial_programming() {
    for input in [Input::Power(false), Input::ResetPin(false)] {
        let mut m = guest::machine();
        m.run_until(Time::from_micros(4000), &[], &mut ()).unwrap();
        let projected = m.firmware();
        assert_ne!(projected[0x9000], 0xff);
        let before = m.snapshot();
        let event = TimedInput {
            at: Time::from_micros(4000),
            input,
        };
        m.run_until(Time::from_micros(12000), &[event], &mut ())
            .unwrap();
        assert_eq!(m.firmware(), projected);
        let mut replay = Machine::from_snapshot(&before);
        replay
            .run_until(Time::from_micros(12000), &[event], &mut ())
            .unwrap();
        state::assert_same_state(&m, &replay);
    }
}
