use crate::optical::Edge;
use anyhow::{bail, ensure, Context, Result};
use hs_core::{Duration, Input, Time, TimedInput};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::Duration as WallDuration,
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

pub(crate) struct Tcp {
    pub state: Arc<Mutex<Connection>>,
    tx: SyncSender<u8>,
    rx: Receiver<u8>,
    stop: Arc<AtomicBool>,
    frame: Option<(Time, u16)>,
    incoming: VecDeque<TimedInput>,
    receive_end: Time,
}

impl Drop for Tcp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Tcp {
    pub fn start(address: String, listen: bool, allow_remote: bool) -> Self {
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
                let result = transport(
                    &address,
                    listen,
                    allow_remote,
                    &worker_stop,
                    &worker_state,
                    outgoing,
                    incoming,
                );
                *worker_state.lock().unwrap() = match result {
                    Ok(()) => Connection::Disconnected,
                    Err(error) => Connection::Error(format!("{error:#}")),
                };
            })
            .expect("create TCP worker");
        Self {
            state,
            tx,
            rx,
            stop,
            frame: None,
            incoming: VecDeque::new(),
            receive_end: Time::ZERO,
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
            let (_, zeros) = self.frame.take().unwrap();
            ensure!(zeros & (1 << 9) == 0, "IR waveform is not 115200 baud 8N1");
            self.tx
                .try_send(!(zeros >> 1) as u8)
                .context("TCP output queue is full or disconnected")?;
        }
        Ok(())
    }

    pub fn inputs(&mut self, now: Time, end: Time) -> Result<Vec<TimedInput>> {
        while self.incoming.len() < CAPACITY * 2 {
            let byte = match self.rx.try_recv() {
                Ok(byte) => byte,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            let start = now.max(self.receive_end);
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
    outgoing: Receiver<u8>,
    incoming: SyncSender<u8>,
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
    socket.set_nonblocking(true)?;
    *state.lock().unwrap() = Connection::Connected;
    let mut pending = VecDeque::<u8>::new();
    let mut bytes = [0; 1024];
    while !stop.load(Ordering::Relaxed) {
        while pending.len() < CAPACITY {
            match outgoing.try_recv() {
                Ok(byte) => pending.push_back(byte),
                Err(_) => break,
            }
        }
        if !pending.is_empty() {
            match socket.write(pending.make_contiguous()) {
                Ok(0) => bail!("Peer closed the connection"),
                Ok(count) => {
                    pending.drain(..count);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(error) => return Err(error.into()),
            }
        }
        match socket.read(&mut bytes) {
            Ok(0) => bail!("Peer closed the connection"),
            Ok(count) => {
                for &byte in &bytes[..count] {
                    incoming
                        .try_send(byte)
                        .context("Incoming IR queue is full; connection interrupted")?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => (),
            Err(error) => return Err(error.into()),
        }
        thread::sleep(WallDuration::from_millis(1));
    }
    Ok(())
}
