//! The channel delay bounds each device's progress before the other catches up.
use crate::{
    args::LinkOptions,
    digest,
    output::{self, Events, RunMetadata},
    timeline,
};
use hs_core::{Duration, Event, Images, Input, Machine, Output, Time, TimedInput};
use std::{collections::VecDeque, fs, ops::ControlFlow, time::Instant};

struct Channel<'a> {
    events: &'a mut Events,
    incoming: &'a mut VecDeque<TimedInput>,
    delay: Duration,
}
impl Output for Channel<'_> {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        if let Event::Infrared { at, emitting } = event {
            match at.checked_add(self.delay) {
                Some(at) => self.incoming.push_back(TimedInput {
                    at,
                    input: Input::InfraredLevel(emitting),
                }),
                None => {
                    self.events.error = Some(std::io::Error::other("optical timestamp overflow"));
                    return ControlFlow::Break(());
                }
            }
        }
        self.events.event(event)
    }
    fn exact_stops(&self) -> bool {
        false
    }
}
fn advance(
    machine: &mut Machine,
    end: Time,
    inputs: &[TimedInput],
    cursor: &mut usize,
    incoming: &mut VecDeque<TimedInput>,
    outgoing: &mut Channel<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let stop = *cursor + inputs[*cursor..].partition_point(|i| i.at < end);
    let mut batch = inputs[*cursor..stop].to_vec();
    while incoming.front().is_some_and(|i| i.at < end) {
        batch.push(incoming.pop_front().unwrap());
    }
    batch.sort_by_key(|i| i.at);
    machine.run_until(end, &batch, outgoing)?;
    outgoing.events.check()?;
    *cursor = stop;
    Ok(())
}
pub fn run(
    primary: &mut Machine,
    events: &mut Events,
    inputs: &[TimedInput],
    end: Time,
    options: &LinkOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let firmware = primary.firmware();
    let eeprom = fs::read(&options.peer_eeprom)?;
    let mut peer = Machine::with_conditions(
        Images {
            firmware: firmware.as_slice(),
            eeprom: &eeprom,
            eeprom_status: 0,
            sensor_nonvolatile: None,
        },
        primary.conditions(),
    )?;
    if peer.now() != primary.now() {
        return Err("linked devices must start at time zero".into());
    }
    let peer_text = options
        .peer_input
        .as_ref()
        .map(fs::read_to_string)
        .transpose()?;
    let peer_inputs = peer_text
        .as_deref()
        .map(timeline::parse)
        .transpose()?
        .unwrap_or_default();
    let firmware_hash = digest::sha256(firmware.as_slice());
    let eeprom_hash = digest::sha256(&eeprom);
    let sensor_hash = digest::sha256(&peer.sensor_nonvolatile());
    let input_hash = peer_text.as_ref().map(|s| digest::sha256(s.as_bytes()));
    let delay = Duration::from_micros(options.delay_us);
    let quantum = Duration::from_micros(
        options
            .run
            .chunk_us
            .map_or(options.delay_us, |v| v.min(options.delay_us)),
    );
    let (mut ab, mut ba) = (VecDeque::new(), VecDeque::new());
    let (mut a_cursor, mut b_cursor) = (0, 0);
    let mut peer_events = Events::default();
    let started = Instant::now();
    while primary.now() < end {
        let to = primary
            .now()
            .checked_add(quantum)
            .ok_or("link horizon overflow")?
            .min(end);
        advance(
            primary,
            to,
            inputs,
            &mut a_cursor,
            &mut ba,
            &mut Channel {
                events,
                incoming: &mut ab,
                delay,
            },
        )?;
        advance(
            &mut peer,
            to,
            &peer_inputs,
            &mut b_cursor,
            &mut ab,
            &mut Channel {
                events: &mut peer_events,
                incoming: &mut ba,
                delay,
            },
        )?;
    }
    if let Some(out) = &options.run.out {
        let dir = out.join("peer");
        fs::create_dir(&dir)?;
        let report = output::report(
            &peer,
            &peer_events,
            started.elapsed().as_secs_f64(),
            &RunMetadata {
                firmware: &firmware_hash,
                initial_eeprom: &eeprom_hash,
                initial_sensor: &sensor_hash,
                initial_eeprom_status: 0,
                input_hash: input_hash.as_deref(),
                chunk_us: Some(options.delay_us),
                start: Time::ZERO,
                requested: end,
                conditions: peer.conditions(),
            },
            None,
        );
        output::export(&peer, &dir, &report)?;
    }
    Ok(())
}
