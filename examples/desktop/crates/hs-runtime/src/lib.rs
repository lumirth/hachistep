mod files;
mod motion;
mod optical;
mod sound;
mod state;
mod storage;
mod tcp;

use anyhow::{ensure, Context, Result};
use files::{Job, Reply};
pub use gilrs::Button as GamepadButton;
pub use hs_core::Buttons;
use hs_core::{Audio, Event, Input, Machine, Output, Time, TimedInput};
use motion::Motion;
use optical::{Edge, Link};
use sound::{Samples, Sound};
pub use state::slot_path;
use state::{PendingSession, RestorePoint};
use std::{
    collections::{BTreeMap, VecDeque},
    ops::ControlFlow,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
pub use tcp::Connection;

pub type Id = u64;
pub const CORE_REVISION: &str = env!("HS_CORE_REVISION");

#[derive(Clone)]
pub struct Player {
    id: Id,
    commands: Sender<(Id, Command)>,
    presentation: Arc<Mutex<Presentation>>,
}

impl Player {
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn send(&self, command: Command) {
        let _ = self.commands.send((self.id, command));
    }
    pub fn presentation(&self) -> Presentation {
        self.presentation.lock().unwrap().clone()
    }
}

#[derive(Clone)]
pub struct Runtime {
    next: Arc<AtomicU64>,
    commands: Sender<(Id, Command)>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        let (commands, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("hachistep-machines".into())
            .spawn(move || Worker::new(receiver).run())
            .expect("create emulation worker");
        Self {
            next: Arc::new(AtomicU64::new(1)),
            commands,
        }
    }
    pub fn player(&self) -> Player {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let presentation = Arc::new(Mutex::new(Presentation::default()));
        let player = Player {
            id,
            commands: self.commands.clone(),
            presentation: presentation.clone(),
        };
        player.send(Command::Attach(presentation));
        player
    }
    pub fn retry_audio(&self) {
        let _ = self.commands.send((0, Command::RetryAudio));
    }
    pub fn gamepad_mapping(&self, mapping: [GamepadButton; 3]) {
        let _ = self.commands.send((0, Command::GamepadMapping(mapping)));
    }
}

pub enum Command {
    Attach(Arc<Mutex<Presentation>>),
    Open {
        firmware: PathBuf,
        eeprom: PathBuf,
        create: bool,
    },
    Buttons(Buttons),
    Focus(bool),
    Release,
    Pause,
    Reset,
    Turbo(bool),
    ToggleTurbo,
    Speed(u16),
    TurboRate(u16),
    Walking(bool),
    Cadence(u16),
    Mute(bool),
    RetryAudio,
    GamepadMapping([gilrs::Button; 3]),
    SaveEepromAs(PathBuf),
    SaveState(PathBuf),
    LoadState(PathBuf),
    UndoLoadState,
    WriteFile {
        path: PathBuf,
        bytes: Vec<u8>,
    },
    RetrySave,
    Link(Id),
    Tcp {
        address: String,
        listen: bool,
        allow_remote: bool,
    },
    Disconnect,
    Close,
    CancelClose,
}

#[derive(Clone)]
#[non_exhaustive]
pub struct Presentation {
    pub pixels: Arc<[u8; 6144]>,
    pub contrast: u8,
    pub ready: bool,
    pub loading: bool,
    pub paused: bool,
    pub can_undo: bool,
    pub slots: [bool; 10],
    pub walking: bool,
    pub cadence: u16,
    pub muted: bool,
    pub speed: f64,
    pub speed_percent: u16,
    pub turbo: bool,
    pub time: Time,
    pub eeprom: Option<PathBuf>,
    pub firmware: String,
    pub peer: Option<Id>,
    pub connection: Connection,
    pub saving: bool,
    pub eeprom_error: Option<String>,
    pub error: Option<String>,
    pub audio_error: Option<String>,
    pub closing: bool,
    pub closed: bool,
}

impl Default for Presentation {
    fn default() -> Self {
        Self {
            pixels: Arc::new([0; 6144]),
            contrast: 32,
            ready: false,
            loading: false,
            paused: false,
            can_undo: false,
            slots: [false; 10],
            walking: false,
            cadence: 120,
            muted: false,
            speed: 0.,
            speed_percent: 100,
            turbo: false,
            time: Time::ZERO,
            eeprom: None,
            firmware: String::new(),
            peer: None,
            connection: Connection::Disconnected,
            saving: false,
            eeprom_error: None,
            error: None,
            audio_error: None,
            closing: false,
            closed: false,
        }
    }
}

impl Presentation {
    pub fn connection_name(&self) -> String {
        if self.peer.is_some() {
            "Local link".into()
        } else {
            self.connection.to_string()
        }
    }
}

struct Session {
    machine: Machine,
    firmware: Arc<Vec<u8>>,
    motion: Motion,
    next_motion: Time,
    reset_release: Option<Time>,
    keyboard: Buttons,
    gamepads: BTreeMap<usize, Buttons>,
    applied_buttons: Buttons,
    inputs: VecDeque<TimedInput>,
    paused: bool,
    fast: bool,
    turbo_toggle: bool,
    speed_percent: u16,
    turbo_percent: u16,
    muted: bool,
    dirty: bool,
    saving: bool,
    closing: bool,
    last_flush: Instant,
    anchor: Instant,
    anchor_time: Time,
    speed_start: (Instant, Time),
    speed: f64,
    emitting: bool,
    sound: Audio,
    samples: Samples,
    tcp: Option<tcp::Tcp>,
}

impl Session {
    fn new(machine: Machine, firmware: Arc<Vec<u8>>, samples: Samples, rate: u32) -> Result<Self> {
        let now = machine.now();
        let wall = Instant::now();
        let sound = machine.audio(rate)?;
        Ok(Self {
            machine,
            firmware,
            motion: Motion::default(),
            next_motion: now,
            reset_release: None,
            keyboard: Buttons::RELEASED,
            gamepads: BTreeMap::new(),
            applied_buttons: Buttons::RELEASED,
            inputs: VecDeque::new(),
            paused: false,
            fast: false,
            turbo_toggle: false,
            speed_percent: 100,
            turbo_percent: 400,
            muted: false,
            dirty: false,
            saving: false,
            closing: false,
            last_flush: wall,
            anchor: wall,
            anchor_time: now,
            speed_start: (wall, now),
            speed: 0.,
            emitting: false,
            sound,
            samples,
            tcp: None,
        })
    }
    fn reanchor(&mut self) {
        self.anchor = Instant::now();
        self.anchor_time = self.machine.now();
    }
    fn clear_audio(&mut self, rate: u32) -> Result<()> {
        self.samples.lock().unwrap().clear();
        self.sound = self.machine.audio(rate)?;
        Ok(())
    }
    fn release(&mut self) {
        self.keyboard = Buttons::RELEASED;
        self.gamepads.clear();
        self.queue_buttons();
    }
    fn queue_buttons(&mut self) {
        let buttons = self.buttons();
        if buttons != self.applied_buttons {
            let now = self.machine.now();
            let mut at = self.target().max(now).min(Time::from_raw(
                now.raw() + hs_core::Duration::from_micros(4000).raw(),
            ));
            if let Some(last) = self.inputs.back() {
                at = at.max(Time::from_raw(last.at.raw() + 1));
            }
            self.inputs.push_back(TimedInput {
                at,
                input: Input::Buttons(buttons),
            });
            self.applied_buttons = buttons;
        }
    }
    fn image(&self) -> storage::Persistent {
        storage::Persistent::capture(&self.machine, storage::hash(&self.firmware))
    }
    fn target(&self) -> Time {
        let micros = self.anchor.elapsed().as_micros().min(u64::MAX as u128) as u64;
        Time::from_raw(
            self.anchor_time.raw()
                + hs_core::Duration::from_micros(micros).raw()
                    * u128::from(if self.fast {
                        self.turbo_percent
                    } else {
                        self.speed_percent
                    })
                    / 100,
        )
    }
    fn buttons(&self) -> Buttons {
        self.gamepads.values().fold(self.keyboard, |a, b| Buttons {
            left: a.left || b.left,
            center: a.center || b.center,
            right: a.right || b.right,
        })
    }
    fn advance(&mut self, end: Time, mut inputs: Vec<TimedInput>) -> Result<Vec<Edge>> {
        let now = self.machine.now();
        let count = self.inputs.partition_point(|input| input.at < end);
        inputs.extend(self.inputs.drain(..count));
        if let Some(at) = self.reset_release.filter(|&at| at < end) {
            inputs.push(TimedInput {
                at: at.max(now),
                input: Input::ResetPin(false),
            });
            self.reset_release = None;
        }
        while self.next_motion < end {
            inputs.push(TimedInput {
                at: self.next_motion.max(now),
                input: Input::Acceleration(self.motion.sample()),
            });
            self.next_motion =
                Time::from_raw(self.next_motion.raw() + hs_core::Duration::from_micros(1000).raw());
        }
        if let Some(tcp) = &mut self.tcp {
            inputs.extend(tcp.inputs(now, end)?);
        }
        inputs.sort_by_key(|input| input.at);
        let mut sink = Sink {
            sound: &mut self.sound,
            samples: Vec::new(),
            edges: Vec::new(),
            dirty: false,
            error: None,
        };
        self.machine.run_until(end, &inputs, &mut sink)?;
        sink.sound.advance(self.machine.now(), &mut |samples| {
            sink.samples.extend_from_slice(samples)
        })?;
        if let Some(error) = sink.error {
            return Err(error);
        }
        self.dirty |= sink.dirty;
        if let Some(edge) = sink.edges.last() {
            self.emitting = edge.emitting;
        }
        if let Some(tcp) = &mut self.tcp {
            if tcp.connected() {
                for edge in &sink.edges {
                    tcp.emit(edge)?;
                }
                tcp.finish(self.machine.now())?;
            }
        }
        if !sink.samples.is_empty()
            && !self.muted
            && !self.fast
            && !self.paused
            && self.speed_percent == 100
        {
            let mut queue = self.samples.lock().unwrap();
            let available = (sink.sound.sample_rate() as usize / 10).saturating_sub(queue.len());
            queue.extend(sink.samples.into_iter().take(available));
        }
        Ok(sink.edges)
    }
    fn capture_state(&self) -> PendingSession {
        PendingSession {
            firmware: storage::hash(&self.firmware),
            snapshot: self.machine.snapshot(),
            motion: self.motion.clone(),
            next_motion: self.next_motion,
            reset_release: self.reset_release,
        }
    }
    fn restore(&mut self, saved: PendingSession, rate: u32, linked: bool) -> Result<()> {
        self.machine = Machine::from_snapshot(&saved.snapshot);
        self.motion = saved.motion;
        self.next_motion = saved.next_motion;
        self.reset_release = saved.reset_release;
        self.paused = true;
        self.fast = false;
        self.turbo_toggle = false;
        self.inputs.clear();
        self.keyboard = Buttons::RELEASED;
        self.gamepads.clear();
        self.applied_buttons = Buttons::RELEASED;
        self.inputs.push_back(TimedInput {
            at: self.machine.now(),
            input: Input::Buttons(Buttons::RELEASED),
        });
        if !linked {
            self.inputs.push_back(TimedInput {
                at: self.machine.now(),
                input: Input::InfraredLevel(false),
            });
        }
        self.dirty = true;
        self.reanchor();
        self.clear_audio(rate)
    }
}

struct Sink<'a> {
    sound: &'a mut Audio,
    samples: Vec<i16>,
    edges: Vec<Edge>,
    dirty: bool,
    error: Option<anyhow::Error>,
}
impl Output for Sink<'_> {
    fn exact_stops(&self) -> bool {
        false
    }
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        match event {
            Event::Buzzer { .. } => {
                if let Err(error) = self.sound.event(event, &mut |samples| {
                    self.samples.extend_from_slice(samples)
                }) {
                    self.error = Some(error.into());
                }
            }
            Event::NvCommit { .. } | Event::NvInterrupted { .. } | Event::NvByte { .. } => {
                self.dirty = true
            }
            Event::Infrared { at, emitting } => self.edges.push(Edge { at, emitting }),
            _ => (),
        }
        ControlFlow::Continue(())
    }
}

struct Worker {
    receive: Receiver<(Id, Command)>,
    views: BTreeMap<Id, Arc<Mutex<Presentation>>>,
    sessions: BTreeMap<Id, Session>,
    links: Vec<Link>,
    undos: BTreeMap<Id, RestorePoint>,
    jobs: files::Files,
    replies: Receiver<Reply>,
    sound: Sound,
    gamepads: Option<gilrs::Gilrs>,
    mapping: [gilrs::Button; 3],
    focused: Option<Id>,
}

impl Worker {
    fn new(receive: Receiver<(Id, Command)>) -> Self {
        let (jobs, replies) = files::start();
        Self {
            receive,
            jobs,
            replies,
            views: BTreeMap::new(),
            sessions: BTreeMap::new(),
            links: Vec::new(),
            undos: BTreeMap::new(),
            sound: Sound::new(),
            gamepads: gilrs::Gilrs::new().ok(),
            mapping: [
                gilrs::Button::DPadLeft,
                gilrs::Button::South,
                gilrs::Button::DPadRight,
            ],
            focused: None,
        }
    }
    fn view(&self, id: Id, update: impl FnOnce(&mut Presentation)) {
        if let Some(view) = self.views.get(&id) {
            update(&mut view.lock().unwrap());
        }
    }
    fn run(&mut self) {
        let mut last = Instant::now();
        let mut publish = Instant::now();
        loop {
            let behind = self.sessions.values().any(|s| {
                !s.paused
                    && s.target().raw().saturating_sub(s.machine.now().raw())
                        > hs_core::Duration::from_micros(2000).raw()
            });
            match self.receive.recv_timeout(if behind {
                Duration::ZERO
            } else {
                Duration::from_millis(1)
            }) {
                Ok((id, command)) => self.dispatch(id, command),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
            for _ in 0..256 {
                match self.receive.try_recv() {
                    Ok((id, command)) => self.dispatch(id, command),
                    Err(_) => break,
                }
            }
            while let Ok(reply) = self.replies.try_recv() {
                self.reply(reply);
            }
            self.poll_gamepads();
            if last.elapsed() > Duration::from_millis(500) {
                let ids: Vec<_> = self.sessions.keys().copied().collect();
                for id in ids {
                    let _ = self.disconnect(id);
                }
                for session in self.sessions.values_mut() {
                    session.release();
                    session.fast = false;
                    session.turbo_toggle = false;
                    session.reanchor();
                    let _ = session.clear_audio(self.sound.rate);
                }
            }
            last = Instant::now();
            self.advance();
            self.flush();
            if publish.elapsed() >= Duration::from_millis(16) {
                self.publish();
                publish = Instant::now();
            }
        }
    }
    fn dispatch(&mut self, id: Id, command: Command) {
        if let Err(error) = self.command(id, command) {
            self.view(id, |view| view.error = Some(format!("{error:#}")));
        }
    }
    fn command(&mut self, id: Id, command: Command) -> Result<()> {
        if !self.sessions.contains_key(&id)
            && matches!(
                command,
                Command::Release | Command::Buttons(_) | Command::Turbo(false)
            )
        {
            return Ok(());
        }
        match command {
            Command::Attach(view) => {
                self.views.insert(id, view);
                return Ok(());
            }
            Command::Open {
                firmware,
                eeprom,
                create,
            } => {
                ensure!(
                    !self.sessions.contains_key(&id),
                    "Close the current EEPROM before opening another"
                );
                self.view(id, |view| {
                    view.loading = true;
                    view.error = None;
                });
                self.jobs.send(Job::Open {
                    id,
                    firmware,
                    eeprom,
                    create,
                })?;
                return Ok(());
            }
            Command::RetryAudio => {
                self.sound = Sound::new();
                for (&id, session) in &mut self.sessions {
                    session.samples = self.sound.add(id);
                    session.clear_audio(self.sound.rate)?;
                }
                return Ok(());
            }
            Command::GamepadMapping(mapping) => {
                if self.mapping != mapping {
                    self.mapping = mapping;
                    for session in self.sessions.values_mut() {
                        session.release();
                    }
                }
                return Ok(());
            }
            Command::Link(peer) => return self.connect(id, peer),
            Command::Disconnect => return self.disconnect(id),
            Command::LoadState(path) => {
                self.jobs.send(Job::Load { id, path })?;
                return Ok(());
            }
            Command::WriteFile { path, bytes } => {
                self.jobs.send(Job::Write { id, path, bytes })?;
                return Ok(());
            }
            Command::UndoLoadState => {
                self.undo_load(id)?;
                self.view(id, |view| view.error = None);
                return Ok(());
            }
            Command::SaveState(path) => {
                let point = self.capture_point(id)?;
                self.jobs.send(Job::Capture {
                    id,
                    path,
                    sessions: point.sessions.into_iter().map(|(_, saved)| saved).collect(),
                    link: point.link,
                })?;
                return Ok(());
            }
            Command::Close => {
                self.disconnect(id)?;
                if let Some(session) = self.sessions.get_mut(&id) {
                    ensure!(!session.machine.eeprom_busy() || !session.paused, "EEPROM programming is paused. Resume to finish, or save an exact state before closing");
                    session.release();
                    session.closing = true;
                } else {
                    self.jobs.send(Job::Close(id))?;
                }
                return Ok(());
            }
            Command::Focus(active) => {
                if active {
                    for session in self.sessions.values_mut() {
                        session.release();
                    }
                    self.focused = Some(id);
                } else if self.focused == Some(id) {
                    self.focused = None;
                }
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.release();
                    session.reanchor();
                    session.clear_audio(self.sound.rate)?;
                }
                return Ok(());
            }
            Command::Pause
            | Command::Turbo(_)
            | Command::ToggleTurbo
            | Command::Speed(_)
            | Command::TurboRate(_) => {
                let ids = self
                    .links
                    .iter()
                    .find(|link| link.ids.contains(&id))
                    .map(|link| link.ids.to_vec())
                    .unwrap_or_else(|| vec![id]);
                let paused = self
                    .sessions
                    .get(&id)
                    .context("Open firmware first")?
                    .paused;
                for id in ids {
                    if let Some(session) = self.sessions.get_mut(&id) {
                        match command {
                            Command::Pause => {
                                session.paused = !paused;
                                session.fast = false;
                                session.turbo_toggle = false;
                                session.release();
                            }
                            Command::Turbo(fast) => {
                                ensure!(
                                    !fast
                                        || !session.tcp.as_ref().is_some_and(|tcp| tcp.connected()),
                                    "Turbo is unavailable during TCP connections"
                                );
                                session.fast = fast || session.turbo_toggle;
                            }
                            Command::ToggleTurbo => {
                                ensure!(
                                    !session.tcp.as_ref().is_some_and(|tcp| tcp.connected()),
                                    "Fast-forward is unavailable during TCP connections"
                                );
                                session.turbo_toggle = !session.turbo_toggle;
                                session.fast = session.turbo_toggle;
                            }
                            Command::Speed(percent) => {
                                ensure!(
                                    (25..=800).contains(&percent),
                                    "Choose a speed between 25% and 800%"
                                );
                                ensure!(
                                    percent == 100
                                        || !session.tcp.as_ref().is_some_and(|tcp| tcp.connected()),
                                    "TCP connections require 100% speed"
                                );
                                session.speed_percent = percent;
                                session.fast = false;
                                session.turbo_toggle = false;
                            }
                            Command::TurboRate(percent) => {
                                ensure!(
                                    (100..=800).contains(&percent),
                                    "Choose a turbo speed between 100% and 800%"
                                );
                                session.turbo_percent = percent;
                            }
                            _ => unreachable!(),
                        }
                        session.reanchor();
                        session.clear_audio(self.sound.rate)?;
                    }
                }
                return Ok(());
            }
            Command::Reset | Command::Tcp { .. } => {
                self.forget_undo(id);
                self.disconnect(id)?;
            }
            _ => (),
        }
        let session = self.sessions.get_mut(&id).context("Open firmware first")?;
        match command {
            Command::Buttons(buttons) => {
                if !session.paused {
                    session.keyboard = buttons;
                    session.queue_buttons();
                }
            }
            Command::Release => {
                session.release();
                session.reanchor();
                session.clear_audio(self.sound.rate)?;
            }
            Command::Reset => {
                session.release();
                session.fast = false;
                session.turbo_toggle = false;
                let now = session.machine.now();
                session.reset_release = None;
                session.advance(
                    Time::from_raw(now.raw() + 1),
                    vec![TimedInput {
                        at: now,
                        input: Input::ResetPin(true),
                    }],
                )?;
                session.reset_release = Some(Time::from_raw(
                    session.machine.now().raw() + hs_core::Duration::from_micros(1000).raw(),
                ));
                session.reanchor();
                session.clear_audio(self.sound.rate)?;
            }
            Command::Walking(walking) => session.motion.walking = walking,
            Command::Cadence(cadence) => {
                ensure!(
                    (30..=240).contains(&cadence),
                    "Cadence must be between 30 and 240 steps/min"
                );
                session.motion.cadence = cadence;
            }
            Command::Mute(muted) => {
                session.muted = muted;
                session.clear_audio(self.sound.rate)?;
            }
            Command::SaveEepromAs(path) => {
                ensure!(
                    !session.saving,
                    "EEPROM programming is in progress. Try Save EEPROM As again when it finishes"
                );
                session.saving = true;
                self.jobs.send(Job::SaveEepromAs {
                    id,
                    path,
                    image: session.image(),
                })?;
                session.dirty = false;
            }
            Command::RetrySave => {
                session.dirty = true;
                session.last_flush = Instant::now() - Duration::from_secs(2);
                self.view(id, |view| view.eeprom_error = None);
            }
            Command::Tcp {
                address,
                listen,
                allow_remote,
            } => {
                session.fast = false;
                session.turbo_toggle = false;
                session.speed_percent = 100;
                session.reanchor();
                session.clear_audio(self.sound.rate)?;
                session.tcp = Some(tcp::Tcp::start(address, listen, allow_remote));
            }
            Command::CancelClose => session.closing = false,
            _ => (),
        }
        self.view(id, |view| view.error = None);
        Ok(())
    }
    fn connect(&mut self, id: Id, peer: Id) -> Result<()> {
        ensure!(id != peer, "Choose another player");
        ensure!(
            !self
                .links
                .iter()
                .any(|l| l.ids.contains(&id) || l.ids.contains(&peer)),
            "A player is already linked"
        );
        let a = self.sessions.get(&id).context("Open firmware first")?;
        let b = self
            .sessions
            .get(&peer)
            .context("The other player closed")?;
        ensure!(
            a.tcp.is_none() && b.tcp.is_none(),
            "Disconnect TCP before linking local players"
        );
        let link = Link::new(
            [id, peer],
            [a.machine.now(), b.machine.now()],
            [a.emitting, b.emitting],
        );
        let paused = a.paused || b.paused;
        let speed = a.speed_percent;
        let turbo = a.turbo_percent;
        for id in [id, peer] {
            let session = self.sessions.get_mut(&id).unwrap();
            session.paused = paused;
            session.fast = false;
            session.turbo_toggle = false;
            session.speed_percent = speed;
            session.turbo_percent = turbo;
            session.reanchor();
            session.clear_audio(self.sound.rate)?;
        }
        self.links.push(link);
        Ok(())
    }
    fn disconnect(&mut self, id: Id) -> Result<()> {
        let ids = if let Some(ix) = self.links.iter().position(|l| l.ids.contains(&id)) {
            self.links.remove(ix).ids.to_vec()
        } else {
            vec![id]
        };
        for id in ids {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.tcp = None;
                let now = session.machine.now();
                session
                    .inputs
                    .retain(|input| !matches!(input.input, Input::InfraredLevel(_)));
                session.inputs.push_back(TimedInput {
                    at: now,
                    input: Input::InfraredLevel(false),
                });
                session
                    .inputs
                    .make_contiguous()
                    .sort_by_key(|input| input.at);
                session.reanchor();
                session.clear_audio(self.sound.rate)?;
            }
        }
        Ok(())
    }
    fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::Opened {
                id,
                machine,
                firmware,
                path,
                slots,
            } => {
                let identity = storage::hash(&firmware)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                match Session::new(*machine, firmware, self.sound.add(id), self.sound.rate) {
                    Ok(session) => {
                        self.sessions.insert(id, session);
                        self.view(id, |view| {
                            view.ready = true;
                            view.loading = false;
                            view.eeprom = Some(path);
                            view.slots = slots;
                            view.firmware = identity;
                        });
                    }
                    Err(error) => self.view(id, |view| {
                        view.loading = false;
                        view.error = Some(error.to_string());
                    }),
                }
            }
            Reply::Saved { id, path, slots } => {
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.saving = false;
                    session.last_flush = Instant::now();
                }
                self.view(id, |view| {
                    view.eeprom = Some(path);
                    view.slots = slots;
                    view.eeprom_error = None;
                });
            }
            Reply::StateWritten { id, slots } => self.view(id, |view| {
                view.slots = slots;
                view.error = None;
            }),
            Reply::Written(id) => self.view(id, |view| view.error = None),
            Reply::Failed {
                id,
                operation,
                error,
            } => {
                if let Some(session) = self.sessions.get_mut(&id) {
                    if operation == "EEPROM" {
                        session.saving = false;
                        session.dirty = true;
                        session.closing = false;
                    }
                }
                self.view(id, |view| {
                    view.loading = false;
                    if operation == "EEPROM" {
                        view.eeprom_error = Some(error);
                    } else {
                        view.error = Some(error);
                    }
                });
            }
            Reply::Closed(id) => {
                self.forget_undo(id);
                self.sessions.remove(&id);
                self.sound.remove(id);
                self.view(id, |view| {
                    view.ready = false;
                    view.closed = true;
                });
                self.views.remove(&id);
            }
            Reply::Loaded { id, capture } => {
                let error = self
                    .restore(id, capture)
                    .err()
                    .map(|error| format!("{error:#}"));
                self.view(id, |view| view.error = error);
            }
        }
    }
    fn advance(&mut self) {
        let linked: Vec<_> = self.links.iter().flat_map(|l| l.ids).collect();
        let ids: Vec<_> = self
            .sessions
            .keys()
            .copied()
            .filter(|id| !linked.contains(id))
            .collect();
        for id in ids {
            let session = self.sessions.get_mut(&id).unwrap();
            if session.paused {
                continue;
            }
            if (session.fast || session.speed_percent != 100)
                && session.tcp.as_ref().is_some_and(|tcp| tcp.connected())
            {
                session.fast = false;
                session.turbo_toggle = false;
                session.speed_percent = 100;
                session.reanchor();
                let _ = session.clear_audio(self.sound.rate);
            }
            let target = session.target();
            let end = Time::from_raw(
                session.machine.now().raw() + hs_core::Duration::from_micros(4000).raw(),
            )
            .min(target);
            if end > session.machine.now() {
                if let Err(error) = session.advance(end, Vec::new()) {
                    session.paused = true;
                    self.view(id, |view| view.error = Some(format!("{error:#}")));
                }
            }
        }
        for ix in 0..self.links.len() {
            let ids = self.links[ix].ids;
            if self.sessions.get(&ids[0]).is_none_or(|s| s.paused) {
                continue;
            }
            let session = &self.sessions[&ids[0]];
            let target = session.target();
            let budget = target
                .raw()
                .saturating_sub(session.machine.now().raw())
                .min(hs_core::Duration::from_micros(4000).raw());
            let end = Time::from_raw(self.links[ix].elapsed.raw() + budget);
            let result = (|| -> Result<()> {
                while self.links[ix].elapsed < end {
                    let to = Time::from_raw(
                        self.links[ix].elapsed.raw() + hs_core::Duration::from_micros(1).raw(),
                    )
                    .min(end);
                    for (side, id) in ids.iter().enumerate() {
                        let inputs = self.links[ix].deliver(side, to);
                        let horizon = Time::from_raw(self.links[ix].origins[side].raw() + to.raw());
                        let edges = self
                            .sessions
                            .get_mut(id)
                            .unwrap()
                            .advance(horizon, inputs)?;
                        self.links[ix].transmit(side, edges.into_iter())?;
                    }
                    self.links[ix].elapsed = to;
                }
                Ok(())
            })();
            if let Err(error) = result {
                for id in ids {
                    self.sessions.get_mut(&id).unwrap().paused = true;
                    self.view(id, |view| view.error = Some(format!("{error:#}")));
                }
            }
        }
    }
    fn flush(&mut self) {
        let mut close = Vec::new();
        for (&id, session) in &mut self.sessions {
            if session.saving || session.machine.eeprom_busy() {
                continue;
            }
            let eeprom_error = self
                .views
                .get(&id)
                .is_some_and(|v| v.lock().unwrap().eeprom_error.is_some());
            if session.dirty
                && !eeprom_error
                && (session.closing || session.last_flush.elapsed() >= Duration::from_millis(500))
            {
                if self
                    .jobs
                    .send(Job::Commit {
                        id,
                        image: session.image(),
                    })
                    .is_ok()
                {
                    session.dirty = false;
                    session.saving = true;
                }
            } else if session.closing && !session.dirty {
                close.push(id);
            }
        }
        for id in close {
            self.sessions.remove(&id);
            let _ = self.jobs.send(Job::Close(id));
        }
    }
    fn publish(&mut self) {
        for (&id, session) in &mut self.sessions {
            let wall = session.speed_start.0.elapsed().as_secs_f64();
            if wall >= 1. {
                session.speed = session
                    .machine
                    .now()
                    .raw()
                    .saturating_sub(session.speed_start.1.raw())
                    as f64
                    / (1u64 << 63) as f64
                    / 2.
                    / wall;
                session.speed_start = (Instant::now(), session.machine.now());
            }
            let mut pixels = [0; 6144];
            session.machine.display(&mut pixels);
            if let Some(view) = self.views.get(&id) {
                let mut view = view.lock().unwrap();
                view.pixels = Arc::new(pixels);
                view.contrast = session.machine.display_contrast();
                view.paused = session.paused;
                view.can_undo = self.undos.values().any(|point| point.contains(id));
                view.walking = session.motion.walking;
                view.cadence = session.motion.cadence;
                view.muted = session.muted;
                view.time = session.machine.now();
                view.speed = session.speed;
                view.speed_percent = session.speed_percent;
                view.turbo = session.fast;
                view.saving = session.saving || session.dirty;
                view.closing = session.closing;
                view.peer = self.links.iter().find(|l| l.ids.contains(&id)).map(|l| {
                    if l.ids[0] == id {
                        l.ids[1]
                    } else {
                        l.ids[0]
                    }
                });
                view.connection = session
                    .tcp
                    .as_ref()
                    .map(|tcp| tcp.state.lock().unwrap().clone())
                    .unwrap_or(Connection::Disconnected);
                view.audio_error = self.sound.error.lock().unwrap().clone();
            }
        }
    }
    fn poll_gamepads(&mut self) {
        let Some(gamepads) = &mut self.gamepads else {
            return;
        };
        while let Some(event) = gamepads.next_event() {
            match event.event {
                gilrs::EventType::Disconnected => {
                    for session in self.sessions.values_mut() {
                        session.gamepads.remove(&usize::from(event.id));
                        session.queue_buttons();
                    }
                }
                gilrs::EventType::ButtonPressed(button, _)
                | gilrs::EventType::ButtonReleased(button, _) => {
                    if let Some(session) = self.focused.and_then(|id| self.sessions.get_mut(&id)) {
                        let pressed = matches!(event.event, gilrs::EventType::ButtonPressed(..));
                        if session.paused {
                            continue;
                        }
                        let buttons = session.gamepads.entry(usize::from(event.id)).or_default();
                        if button == self.mapping[0] {
                            buttons.left = pressed;
                        }
                        if button == self.mapping[1] {
                            buttons.center = pressed;
                        }
                        if button == self.mapping[2] {
                            buttons.right = pressed;
                        }
                        session.queue_buttons();
                    }
                }
                _ => (),
            }
        }
    }
}
