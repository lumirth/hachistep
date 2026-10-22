use crate::optical::Edge;
use anyhow::{ensure, Context, Result};
use hs_core::{Duration, Input, Time, TimedInput};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration as WallDuration, Instant},
};

// pw IrConfigure: 3.6864 MHz, BRR=0, SEMR=0, SMR=0 gives 115200 baud, 8N1 SIR.
// Peers send raw bytes: h4lfheart/pocketwalker@86026a0 and Ceravia/melonDS-IR@c5ce161.
const BIT: u128 = (1u128 << 64) / 115_200;
const CAPACITY: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Disconnected,
    Listening,
    Connecting,
    Connected,
    Error(String),
}

impl Connection {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Listening | Self::Connecting | Self::Connected)
    }
}

impl std::fmt::Display for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("Disconnected"),
            Self::Listening => f.write_str("Listening"),
            Self::Connecting => f.write_str("Connecting"),
            Self::Connected => f.write_str("Connected"),
            Self::Error(error) => write!(f, "Connection error: {error}"),
        }
    }
}

pub(crate) struct Incoming {
    pub(crate) bytes: SyncSender<(Instant, u8)>,
    pub(crate) wake: thread::Thread,
}

pub(crate) struct Tcp {
    pub melon_ds: bool,
    pub state: Arc<Mutex<Connection>>,
    tx: SyncSender<Vec<u8>>,
    rx: Receiver<(Instant, u8)>,
    stop: Arc<AtomicBool>,
    frame: Option<(Time, u16)>,
    incoming: VecDeque<TimedInput>,
    receive_end: Time,
    burst: Vec<u8>,
    transmit_end: Time,
}

impl Drop for Tcp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Tcp {
    pub fn start(address: String, listen: bool, allow_remote: bool, wake: thread::Thread) -> Self {
        Self::spawn(address, listen, allow_remote, None, wake)
    }

    pub fn melon_ds(port: u16, wake: thread::Thread) -> Self {
        Self::spawn(String::new(), false, false, Some(port), wake)
    }

    fn spawn(
        address: String,
        listen: bool,
        allow_remote: bool,
        melon_port: Option<u16>,
        wake: thread::Thread,
    ) -> Self {
        let (tx, outgoing) = mpsc::sync_channel(CAPACITY);
        let (incoming, rx) = mpsc::sync_channel(CAPACITY);
        let state = Arc::new(Mutex::new(if listen {
            Connection::Listening
        } else {
            Connection::Connecting
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        thread::Builder::new()
            .name("hachistep-tcp".into())
            .spawn(move || {
                let incoming = Incoming {
                    bytes: incoming,
                    wake,
                };
                let result = if let Some(port) = melon_port {
                    crate::melonds::transport(port, &worker_stop, &worker_state, outgoing, incoming)
                } else {
                    transport(
                        &address,
                        listen,
                        allow_remote,
                        &worker_stop,
                        &worker_state,
                        outgoing,
                        incoming,
                    )
                };
                *worker_state.lock().unwrap() = match result {
                    Ok(()) => Connection::Disconnected,
                    Err(error) => Connection::Error(format!("{error:#}")),
                };
            })
            .expect("create TCP worker");
        Self {
            melon_ds: melon_port.is_some(),
            state,
            tx,
            rx,
            stop,
            frame: None,
            incoming: VecDeque::new(),
            receive_end: Time::ZERO,
            burst: Vec::new(),
            transmit_end: Time::ZERO,
        }
    }

    pub fn active(&self) -> bool {
        self.state.lock().unwrap().is_active()
    }

    pub fn connected(&self) -> bool {
        *self.state.lock().unwrap() == Connection::Connected
    }

    pub fn emit(&mut self, edge: &Edge) -> Result<()> {
        self.finish(edge.at)?;
        if !edge.emitting {
            return Ok(());
        }
        match &mut self.frame {
            None => self.frame = Some((edge.at, 1)),
            Some((start, zeros)) => {
                let cell = (edge.at.raw() - start.raw() + BIT / 2) / BIT;
                if cell < 10 {
                    *zeros |= 1 << cell;
                }
            }
        }
        Ok(())
    }

    pub fn finish(&mut self, now: Time) -> Result<()> {
        if self
            .frame
            .is_some_and(|(start, _)| now.raw() >= start.raw() + BIT * 10)
        {
            let (start, zeros) = self.frame.take().unwrap();
            ensure!(zeros & (1 << 9) == 0, "IR waveform is not 115200 baud 8N1");
            self.burst.push(!(zeros >> 1) as u8);
            self.transmit_end = Time::from_raw(start.raw() + BIT * 10);
        }
        // Peers frame bursts by idle time, so keep a guest burst in one socket write.
        if !self.burst.is_empty()
            && self.frame.is_none()
            && (now.raw() >= self.transmit_end.raw() + BIT * 3 || self.burst.len() >= CAPACITY)
        {
            self.tx
                .try_send(std::mem::take(&mut self.burst))
                .context("TCP output queue is full or disconnected")?;
        }
        Ok(())
    }

    pub fn inputs(
        &mut self,
        now: Time,
        end: Time,
        anchor: Instant,
        anchor_time: Time,
    ) -> Result<Vec<TimedInput>> {
        while self.incoming.len() < CAPACITY * 2 {
            let (arrived, byte) = match self.rx.try_recv() {
                Ok(byte) => byte,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            let elapsed = arrived
                .saturating_duration_since(anchor)
                .as_micros()
                .min(u64::MAX as u128) as u64;
            let arrived = anchor_time
                .checked_add(Duration::from_micros(elapsed))
                .context("IR clock overflow")?;
            let start = now.max(arrived).max(self.receive_end);
            for cell in 0..9 {
                if cell == 0 || byte & (1 << (cell - 1)) == 0 {
                    let at = Time::from_raw(start.raw() + BIT * cell);
                    self.incoming.push_back(TimedInput {
                        at,
                        input: Input::InfraredLevel(true),
                    });
                    self.incoming.push_back(TimedInput {
                        at: Time::from_raw(at.raw() + BIT * 3 / 16),
                        input: Input::InfraredLevel(false),
                    });
                }
            }
            self.receive_end = start
                .checked_add(Duration::from_raw(BIT * 10))
                .context("IR clock overflow")?;
        }
        let count = self.incoming.partition_point(|input| input.at < end);
        Ok(self.incoming.drain(..count).collect())
    }
}

fn transport(
    address: &str,
    listen: bool,
    allow_remote: bool,
    stop: &AtomicBool,
    state: &Mutex<Connection>,
    outgoing: Receiver<Vec<u8>>,
    incoming: Incoming,
) -> Result<()> {
    let mut socket = if listen {
        let address: SocketAddr = address
            .parse()
            .context("Use a numeric listening address and port")?;
        ensure!(
            address.ip().is_loopback() || allow_remote,
            "Enable Allow remote connections before listening beyond this computer"
        );
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(WallDuration::from_millis(5))
                }
                Err(error) => return Err(error.into()),
            }
        }
    } else {
        let addresses: Vec<_> = address.to_socket_addrs()?.collect();
        let mut connected = None;
        for address in addresses {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if let Ok(socket) = TcpStream::connect_timeout(&address, WallDuration::from_secs(2)) {
                connected = Some(socket);
                break;
            }
        }
        connected.context("Could not connect to peer")?
    };
    socket.set_nodelay(true)?;
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(WallDuration::from_millis(50)))?;
    socket.set_write_timeout(Some(WallDuration::from_millis(50)))?;
    let mut writer = socket.try_clone()?;
    *state.lock().unwrap() = Connection::Connected;
    thread::scope(|scope| {
        let transmit = scope.spawn(move || {
            let sent = write_bytes(&mut writer, outgoing, stop);
            stop.store(true, Ordering::Relaxed);
            let _ = writer.shutdown(Shutdown::Both);
            sent
        });
        let receive = (|| -> Result<()> {
            let mut bytes = [0; 1024];
            while !stop.load(Ordering::Relaxed) {
                match socket.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        let arrived = Instant::now();
                        for &byte in &bytes[..count] {
                            incoming
                                .bytes
                                .try_send((arrived, byte))
                                .context("Incoming IR queue is full; connection interrupted")?;
                        }
                        incoming.wake.unpark();
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        })();
        stop.store(true, Ordering::Relaxed);
        let _ = socket.shutdown(Shutdown::Both);
        let sent = transmit
            .join()
            .map_err(|_| anyhow::anyhow!("TCP writer stopped unexpectedly"))?;
        sent.and(receive)
    })
}

fn write_bytes(
    socket: &mut TcpStream,
    outgoing: Receiver<Vec<u8>>,
    stop: &AtomicBool,
) -> Result<()> {
    while !stop.load(Ordering::Relaxed) {
        let bytes = match outgoing.recv_timeout(WallDuration::from_millis(50)) {
            Ok(byte) => byte,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        socket.write_all(&bytes)?;
    }
    Ok(())
}
