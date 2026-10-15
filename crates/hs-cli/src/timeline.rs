use hs_core::{Acceleration, AnalogPin, Buttons, DigitalPin, Input, Time, TimedInput};
use std::error::Error;

const POWER: u128 = key("power");
const BUTTONS: u128 = key("buttons");
const ACCEL: u128 = key("accel");
const SUPPLY: u128 = key("supply");
const TEMPERATURE: u128 = key("temperature");
const IR: u128 = key("ir");
const RESET: u128 = key("reset");
const NMI: u128 = key("nmi");
const DIGITAL: u128 = key("digital");
const ANALOG: u128 = key("analog");

/// Human-editable deterministic physical inputs. No firmware state is modified.
pub fn parse(text: &str) -> Result<Vec<TimedInput>, Box<dyn Error>> {
    // Most lines take a dozen bytes or more.
    let mut result = Vec::with_capacity(text.len() / 12);
    let mut prior = None;
    let mut seen = 0u32;
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut line_no = 0;
    while start < bytes.len() {
        // One line, its comment cut, split at its commas in the same pass.
        // No kind takes more than five fields; a sixth only fails the match.
        let mut ends = [0usize; 6];
        let mut count = 0;
        let mut end = start;
        let mut cut = None;
        while end < bytes.len() && bytes[end] != b'\n' {
            match bytes[end] {
                b'#' if cut.is_none() => cut = Some(end),
                b',' if cut.is_none() => {
                    if count < ends.len() {
                        ends[count] = end;
                    }
                    count += 1;
                }
                _ => {}
            }
            end += 1;
        }
        let line_start = start;
        let line_end = cut.unwrap_or(end);
        start = end + 1;
        line_no += 1;
        if trim(&text[line_start..line_end]).is_empty() {
            continue;
        }
        let mut fields = [""; 6];
        let mut from = line_start;
        for (i, field) in fields.iter_mut().enumerate().take(count + 1) {
            let to = if i < count { ends[i] } else { line_end };
            *field = trim(&text[from..to]);
            from = to + 1;
        }
        count += 1;
        let line_no = line_no - 1;
        let p = &fields[..count.min(fields.len())];
        let fail = |message: &str| format!("timeline line {}: {message}", line_no + 1);
        if count < 3 {
            return Err(fail("expected time_us,kind,values...").into());
        }
        let at =
            Time::from_micros(micros(p[0]).ok_or_else(|| fail("invalid microsecond timestamp"))?);
        let bit = |s: &str| -> Result<bool, Box<dyn Error>> {
            match s.as_bytes() {
                [b'0'] => Ok(false),
                [b'1'] => Ok(true),
                _ => Err(fail("Boolean values must be 0 or 1").into()),
            }
        };
        let (input, tag) = match (key(p[1]), count) {
            (POWER, 3) => (Input::Power(bit(p[2])?), 22),
            (BUTTONS, 5) => (
                Input::Buttons(Buttons {
                    left: bit(p[2])?,
                    center: bit(p[3])?,
                    right: bit(p[4])?,
                }),
                0,
            ),
            (ACCEL, 5) => (
                Input::Acceleration(Acceleration {
                    x: p[2].parse()?,
                    y: p[3].parse()?,
                    z: p[4].parse()?,
                }),
                1,
            ),
            (SUPPLY, 3) => (Input::SupplyMillivolts(p[2].parse()?), 2),
            (TEMPERATURE, 3) => (Input::TemperatureMillicelsius(p[2].parse()?), 23),
            (IR, 3) => (Input::InfraredLevel(bit(p[2])?), 3),
            (RESET, 3) => (Input::ResetPin(bit(p[2])?), 4),
            (NMI, 3) => (Input::NmiPin(bit(p[2])?), 31),
            (DIGITAL, 4) => {
                let pin = match p[2] {
                    "p10" => DigitalPin::P10,
                    "p11" => DigitalPin::P11,
                    "p12" => DigitalPin::P12,
                    "p30" => DigitalPin::P30,
                    "p31" => DigitalPin::P31,
                    "p32" => DigitalPin::P32,
                    "p90" => DigitalPin::P90,
                    "p91" => DigitalPin::P91,
                    "p92" => DigitalPin::P92,
                    "p93" => DigitalPin::P93,
                    "adtrg" => DigitalPin::Adtrg,
                    _ => return Err(fail("unknown digital fixture pin").into()),
                };
                let level = if p[3] == "release" {
                    None
                } else {
                    Some(bit(p[3])?)
                };
                (Input::DigitalPin { pin, level }, 12 + pin.index())
            }
            (ANALOG, 4) => {
                let pin = match p[2] {
                    "pb0" => AnalogPin::Pb0,
                    "pb1" => AnalogPin::Pb1,
                    "pb2" => AnalogPin::Pb2,
                    "pb3" => AnalogPin::Pb3,
                    "pb4" => AnalogPin::Pb4,
                    "pb5" => AnalogPin::Pb5,
                    "vcref" => AnalogPin::Vcref,
                    _ => return Err(fail("unknown analog package pin").into()),
                };
                let millivolts = if p[3] == "release" {
                    None
                } else {
                    Some(p[3].parse()?)
                };
                (Input::AnalogPin { pin, millivolts }, 5 + pin.index())
            }
            _ => return Err(fail("unknown input kind or incorrect number of values").into()),
        };
        if let Some(old) = prior {
            if at < old {
                return Err(fail("timestamps must be ordered").into());
            }
            if at != old {
                seen = 0;
            }
        }
        if seen & (1 << tag) != 0 {
            return Err(fail("duplicate input property at the same timestamp").into());
        }
        seen |= 1 << tag;
        prior = Some(at);
        result.push(TimedInput { at, input });
    }
    Ok(result)
}

/// `str::trim`, without its search when the ends are not whitespace.
fn trim(s: &str) -> &str {
    match s.as_bytes() {
        [first, .., last] if first.is_ascii_graphic() && last.is_ascii_graphic() => s,
        [only] if only.is_ascii_graphic() => s,
        _ => s.trim(),
    }
}

/// A decimal timestamp, as `u64::from_str` reads one.
fn micros(s: &str) -> Option<u64> {
    let digits = s.strip_prefix('+').unwrap_or(s).as_bytes();
    if digits.is_empty() {
        return None;
    }
    digits.iter().try_fold(0u64, |value, &b| {
        b.is_ascii_digit()
            .then(|| value.checked_mul(10)?.checked_add(u64::from(b - b'0')))
            .flatten()
    })
}

/// A kind of at most 16 bytes as one integer, so that matching it compares no
/// strings; longer kinds match none.
const fn key(s: &str) -> u128 {
    let bytes = s.as_bytes();
    if bytes.len() > 16 {
        return u128::MAX;
    }
    let mut value = 0u128;
    let mut i = 0;
    while i < bytes.len() {
        value |= (bytes[i] as u128) << (8 * i);
        i += 1;
    }
    value
}
