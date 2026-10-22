use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// melonDS 1.1 has a 1152-byte packet buffer and a 1024-byte *hex* read buffer.
// Keep memory requests at 512 bytes, use one outstanding request, and never
// retry a write whose reply was lost.
pub(super) struct Rsp {
    input: BufReader<TcpStream>,
    output: TcpStream,
    halted: bool,
    breakpoint: Option<u32>,
    trace: Option<(PathBuf, VecDeque<String>)>,
}

impl Rsp {
    pub fn connect(port: u16) -> Result<Self> {
        let mut output = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            Duration::from_secs(2),
        )
        .context("Could not reach melonDS. Enable its ARM9 GDB stub, restart melonDS, and open your game")?;
        output.set_nodelay(true)?;
        output.set_read_timeout(Some(Duration::from_secs(2)))?;
        output.set_write_timeout(Some(Duration::from_secs(2)))?;
        output.write_all(b"+")?;
        let mut this = Self {
            input: BufReader::new(output.try_clone()?),
            output,
            halted: false,
            breakpoint: None,
            trace: std::env::var_os("HACHISTEP_MELONDS_TRACE").map(|p| (p.into(), VecDeque::new())),
        };
        // NoAck survives disconnect in melonDS 1.1: the initial '+' reply may
        // be absent on reconnect. Send the negotiation without waiting for it.
        this.ok("QStartNoAckMode")?;
        ensure!(
            this.request("qHostInfo")?
                .split(';')
                .any(|field| field == "cpusubtype:7"),
            "Choose melonDS’s ARM9 debugger port"
        );
        this.halt()?;
        Ok(this)
    }
    pub fn trace(&mut self, event: &str, detail: impl FnOnce() -> String) {
        if let Some((_, lines)) = &mut self.trace {
            if lines.len() == 65536 {
                lines.pop_front();
            }
            lines.push_back(format!(
                "{:.6} {event} {}\n",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64(),
                detail()
            ));
        }
    }
    fn byte(&mut self) -> Result<u8> {
        let mut byte = [0];
        if let Err(error) = self.input.read_exact(&mut byte) {
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) {
                bail!("melonDS stopped responding. Resume it or close its settings dialog, then retry");
            }
            return Err(error).context("Lost the melonDS debugger connection");
        }
        Ok(byte[0])
    }
    fn receive(&mut self) -> Result<String> {
        loop {
            match self.byte()? {
                b'$' => break,
                b'+' => (),
                b'-' => bail!("melonDS rejected a debugger packet"),
                _ => bail!("Unexpected debugger response from melonDS"),
            }
        }
        let mut body = Vec::new();
        loop {
            let byte = self.byte()?;
            if byte == b'#' {
                break;
            }
            ensure!(body.len() < 1152, "Oversized debugger response");
            body.push(byte);
        }
        let checksum = [self.byte()?, self.byte()?];
        let checksum = unhex(std::str::from_utf8(&checksum)?)?[0];
        ensure!(
            checksum == body.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)),
            "Debugger checksum mismatch"
        );
        // NoAck mode disables protocol retransmission. Stock still accepts '+',
        // and sending it promptly acknowledges the TCP data as well, preventing
        // its next small response from waiting behind Nagle on every platform.
        self.output.write_all(b"+")?;
        Ok(String::from_utf8(body)?)
    }
    fn send(&mut self, body: &str) -> Result<()> {
        ensure!(body.len() + 4 < 1152, "Debugger request is too large");
        let checksum = body.bytes().fold(0u8, u8::wrapping_add);
        self.output
            .write_all(format!("${body}#{checksum:02x}").as_bytes())?;
        Ok(())
    }
    pub fn request(&mut self, body: &str) -> Result<String> {
        let start = Instant::now();
        self.send(body)?;
        let received = loop {
            match self.receive() {
                Ok(stop) if stop == "S02" || stop == "S05" => {
                    self.halted = true;
                }
                other => break other,
            }
        };
        self.trace("request", || {
            format!(
                "{} {:.3}ms {}",
                body.split(':').next().unwrap_or(body),
                start.elapsed().as_secs_f64() * 1000.,
                if received.is_ok() { "ok" } else { "error" }
            )
        });
        let reply = received?;
        ensure!(!reply.starts_with('E'), "melonDS debugger error: {reply}");
        Ok(reply)
    }
    pub fn ok(&mut self, body: &str) -> Result<()> {
        ensure!(
            self.request(body)? == "OK",
            "Unsupported melonDS debugger command"
        );
        Ok(())
    }
    pub fn halt(&mut self) -> Result<()> {
        if !self.halted {
            if self.wait_stop()? {
                return Ok(());
            }
            let start = Instant::now();
            self.output.write_all(&[3])?;
            let mut wake = Instant::now();
            while !self.wait_stop()? {
                ensure!(start.elapsed() < Duration::from_secs(2), "melonDS stopped responding. Resume it or close its settings dialog, then retry");
                // Stock only checks its receive buffer when the socket becomes
                // readable. A harmless ACK wakes it if interrupt and continue
                // arrived together and the interrupt remained buffered.
                if wake.elapsed() >= Duration::from_millis(10) {
                    self.output.write_all(b"+")?;
                    wake = Instant::now();
                }
            }
            self.trace("halt", || {
                format!("{:.3}ms ok", start.elapsed().as_secs_f64() * 1000.)
            });
        }
        Ok(())
    }
    pub fn breakpoint(&mut self, address: u32) -> Result<()> {
        self.ok(&format!("Z1,{address:x},4"))?;
        self.breakpoint = Some(address);
        Ok(())
    }
    pub fn wait_stop(&mut self) -> Result<bool> {
        if self.halted {
            return Ok(true);
        }
        self.output
            .set_read_timeout(Some(Duration::from_millis(1)))?;
        let available = self.input.fill_buf();
        let ready = match available {
            Ok(bytes) => {
                ensure!(!bytes.is_empty(), "melonDS disconnected");
                true
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                false
            }
            Err(e) => return Err(e.into()),
        };
        self.output.set_read_timeout(Some(Duration::from_secs(2)))?;
        if ready {
            let stopped = self.receive()?;
            self.trace("stop", || stopped.clone());
            ensure!(
                stopped == "S02" || stopped == "S05",
                "Unexpected stop: {stopped}"
            );
            self.halted = true;
        }
        Ok(ready)
    }
    pub fn resume(&mut self) -> Result<()> {
        // Standard continue has no reply. vCont;c adds an unnecessary OK
        // before the next stop, exposing stock's Nagle/delayed-ACK interaction.
        self.send("c")?;
        self.halted = false;
        Ok(())
    }
    pub fn read(&mut self, address: u32, length: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(length);
        while bytes.len() < length {
            let count = (length - bytes.len()).min(512);
            let reply = self.request(&format!("m{:x},{count:x}", address + bytes.len() as u32))?;
            let part = unhex(&reply)?;
            ensure!(part.len() == count, "Incomplete melonDS memory read");
            bytes.extend(part);
        }
        Ok(bytes)
    }
    pub fn write(&mut self, address: u32, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() <= 512,
            "Memory write must fit one debugger packet"
        );
        self.ok(&format!("M{address:x},{:x}:{}", bytes.len(), hex(bytes)))
    }
}

impl Drop for Rsp {
    fn drop(&mut self) {
        if let Some(address) = self.breakpoint {
            let _ = self.ok(&format!("z1,{address:x},4"));
        }
        // Stock melonDS retains breakpoints after socket loss. Remove ours
        // before releasing execution on an ordinary disconnect.
        let _ = self.send("c");
        if let Some((path, lines)) = &self.trace {
            if let Ok(mut file) = std::fs::File::create(path) {
                for line in lines {
                    let _ = file.write_all(line.as_bytes());
                }
            }
        }
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    text
}
pub(super) fn unhex(text: &str) -> Result<Vec<u8>> {
    ensure!(
        text.len().is_multiple_of(2) && text.is_ascii(),
        "Malformed debugger hex data"
    );
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).context("Malformed debugger hex data"))
        .collect()
}
