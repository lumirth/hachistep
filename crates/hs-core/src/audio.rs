//! Streaming mono audio from timed buzzer drive changes. The renderer owns
//! resampling history; the emulated machine runs independently of playback.
use crate::{signals::Piezo, Error, Event, Time};
use blip_buf::BlipBuf;

const PHASES: u32 = 4096;
const BLOCK: usize = 256;
const LEVEL: i32 = 8192;

/// Converts buzzer events into band-limited signed 16-bit PCM. Construction
/// allocates a fixed buffer; rendering streams borrowed sample blocks.
///
/// Feed every buzzer event in order, then call `advance` with the returned run
/// horizon. Playback, gain and device buffering belong to the caller. Recreate
/// this renderer after loading a machine state or changing the sample rate.
pub struct Audio {
    buffer: BlipBuf,
    sample_rate: u32,
    origin: Time,
    now: Time,
    clocks: u128,
    level: i32,
}
impl Audio {
    /// Start a new sample stream at `at`, with the current piezo drive.
    pub fn new(sample_rate: u32, at: Time, drive: Piezo) -> Result<Self, Error> {
        if !(1000..=192_000).contains(&sample_rate) {
            return Err(Error::BadInput(
                "audio rate must be between 1000 and 192000 Hz",
            ));
        }
        let mut buffer = BlipBuf::new(BLOCK as u32);
        // This power-of-two ratio is exact. Convert the hardware timeline to
        // this grid once per transition, retaining the origin across run calls.
        buffer
            .set_rates(f64::from(PHASES), 1.0)
            .map_err(Error::Internal)?;
        buffer.clear();
        let level = level(drive);
        buffer.add_delta(0, level).map_err(Error::Internal)?;
        Ok(Self {
            buffer,
            sample_rate,
            origin: at,
            now: at,
            clocks: 0,
            level,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    /// Consume a hardware event. Events unrelated to the buzzer are ignored.
    pub fn event(&mut self, event: Event, samples: &mut impl FnMut(&[i16])) -> Result<(), Error> {
        if let Event::Buzzer { at, drive } = event {
            self.advance(at, samples)?;
            let next = level(drive);
            self.buffer
                .add_delta(0, next - self.level)
                .map_err(Error::Internal)?;
            self.level = next;
        }
        Ok(())
    }
    /// Render through a completed machine horizon, including silent intervals.
    /// Keep the partial sample and filter history for the next call.
    pub fn advance(&mut self, end: Time, samples: &mut impl FnMut(&[i16])) -> Result<(), Error> {
        if end < self.now {
            return Err(Error::PastInput {
                now: self.now,
                requested: end,
            });
        }
        let elapsed = end.raw() - self.origin.raw();
        let frequency = u128::from(self.sample_rate) * u128::from(PHASES);
        let target = (elapsed >> 64) * frequency
            + (((elapsed & u128::from(u64::MAX)) * frequency + (1 << 63)) >> 64);
        let mut block = [0; BLOCK];
        while self.clocks < target {
            let count = (target - self.clocks).min((BLOCK as u128) * u128::from(PHASES));
            self.buffer
                .end_frame(count as u32)
                .map_err(Error::Internal)?;
            self.clocks += count;
            let count = self.buffer.read_samples(&mut block, false);
            if count != 0 {
                samples(&block[..count]);
            }
        }
        self.now = end;
        Ok(())
    }
}

fn level(drive: Piezo) -> i32 {
    match drive {
        Piezo::Negative => -LEVEL,
        Piezo::Neutral => 0,
        Piezo::Positive => LEVEL,
    }
}
