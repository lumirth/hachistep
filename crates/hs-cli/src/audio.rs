use hs_core::{Audio, Event, Machine, Time};
use std::{
    fs,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::Path,
};

pub struct Capture {
    renderer: Audio,
    writer: BufWriter<fs::File>,
    bytes: u32,
    error: Option<io::Error>,
}
fn append(
    writer: &mut BufWriter<fs::File>,
    bytes: &mut u32,
    error: &mut Option<io::Error>,
    samples: &[i16],
) {
    if error.is_some() {
        return;
    }
    let result = (|| {
        let size = u32::try_from(
            samples
                .len()
                .checked_mul(2)
                .ok_or_else(|| io::Error::other("audio size overflow"))?,
        )
        .map_err(io::Error::other)?;
        let total = bytes
            .checked_add(size)
            .filter(|v| *v <= u32::MAX - 36)
            .ok_or_else(|| io::Error::other("audio exceeds WAV size limit"))?;
        for sample in samples {
            writer.write_all(&sample.to_le_bytes())?;
        }
        *bytes = total;
        Ok(())
    })();
    *error = result.err();
}
impl Capture {
    pub fn new(machine: &Machine, path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let renderer = machine.audio(48_000)?;
        let mut writer = BufWriter::new(
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)?,
        );
        writer.write_all(b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0")?;
        writer.write_all(&48_000u32.to_le_bytes())?;
        writer.write_all(&96_000u32.to_le_bytes())?;
        writer.write_all(b"\x02\0\x10\0data\0\0\0\0")?;
        Ok(Self {
            renderer,
            writer,
            bytes: 0,
            error: None,
        })
    }
    pub fn event(&mut self, event: Event) -> io::Result<()> {
        let Self {
            renderer,
            writer,
            bytes,
            error,
        } = self;
        renderer
            .event(event, &mut |block| append(writer, bytes, error, block))
            .map_err(io::Error::other)?;
        error.take().map_or(Ok(()), Err)
    }
    pub fn finish(mut self, end: Time) -> Result<(), Box<dyn std::error::Error>> {
        let Self {
            renderer,
            writer,
            bytes,
            error,
        } = &mut self;
        renderer.advance(end, &mut |block| append(writer, bytes, error, block))?;
        if let Some(e) = error.take() {
            return Err(e.into());
        }
        writer.seek(SeekFrom::Start(4))?;
        writer.write_all(&(*bytes + 36).to_le_bytes())?;
        writer.seek(SeekFrom::Start(40))?;
        writer.write_all(&bytes.to_le_bytes())?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        Ok(())
    }
}
