mod files;
mod melonds;
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
    worker: thread::Thread,
    presentation: Arc<Mutex<Presentation>>,
}

impl Player {
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn send(&self, command: Command) {
        let _ = self.commands.send((self.id, command));
        self.worker.unpark();
    }
    pub fn presentation(&self) -> Presentation {
        self.presentation.lock().unwrap().clone()
    }
}

#[derive(Clone)]
pub struct Runtime {
    next: Arc<AtomicU64>,
    commands: Sender<(Id, Command)>,
    worker: thread::Thread,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        let (commands, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("hachistep-machines".into())
            .spawn(move || Worker::new(receiver).run())
            .expect("create emulation worker")
            .thread()
            .clone();
        Self {
            next: Arc::new(AtomicU64::new(1)),
            commands,
            worker,
        }
    }
    pub fn player(&self) -> Player {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let presentation = Arc::new(Mutex::new(Presentation::default()));
        let player = Player {
            id,
            commands: self.commands.clone(),
            worker: self.worker.clone(),
            presentation: presentation.clone(),
        };
        player.send(Command::Attach(presentation));
        player
    }
    pub fn sound_enabled(&self, enabled: bool) {
        let _ = self.commands.send((0, Command::Sound(enabled)));
        self.worker.unpark();
    }
    pub fn retry_audio(&self) {
        let _ = self.commands.send((0, Command::RetryAudio));
        self.worker.unpark();
    }
    pub fn gamepad_mapping(&self, mapping: [GamepadButton; 3]) {
        let _ = self.commands.send((0, Command::GamepadMapping(mapping)));
        self.worker.unpark();
    }
}

pub enum Command {
    Attach(Arc<Mutex<Presentation>>),
    Open {
        firmware: PathBuf,
        eeprom: Option<PathBuf>,
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
    Sound(bool),
    GamepadMapping([gilrs::Button; 3]),
    SaveEepromAs(PathBuf),
    SaveState(PathBuf),
    LoadState(PathBuf),
    UndoLoadState,
    WriteFile {
        path: PathBuf,
        bytes: Vec<u8>,
    },
    SaveEeprom,
    PrepareClose,
    Link(Id),
    Tcp {
        address: String,
        listen: bool,
        allow_remote: bool,
    },
    MelonDs {
        port: u16,
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
    pub slots: [Option<u64>; 10],
    pub walking: bool,
    pub cadence: u16,
    pub muted: bool,
    pub speed: f64,
    pub speed_percent: u16,
    pub turbo_percent: u16,
    pub turbo: bool,
    pub time: Time,
    pub eeprom: Option<PathBuf>,
    pub firmware: String,
    pub peer: Option<Id>,
    pub connection: Connection,
    pub connection_address: Option<String>,
    pub melon_ds: bool,
    pub saving: bool,
    pub modified: bool,
    pub eeprom_error: Option<String>,
    pub error: Option<String>,
    pub audio_error: Option<String>,
    pub audio_resets: u64,
    pub audio_buffered: usize,
    pub audio_rate: u32,
    pub connection_error: Option<String>,
    pub audio_underruns: u64,
    pub motion_samples: Arc<[hs_core::Acceleration]>,
    pub activity: Arc<[InfraredActivity]>,
    pub display_enabled: bool,
    pub closing: bool,
    pub closed: bool,
    pub completion: Option<(u64, Completion)>,
    pub gamepad_press: Option<(u64, GamepadButton)>,
}

#[derive(Clone, Copy)]
pub struct InfraredActivity {
    pub at: Time,
    pub sent: u32,
    pub received: u32,
}

#[derive(Clone)]
pub enum Completion {
    SavedEeprom(PathBuf),
    SavedState(PathBuf),
    LoadedState,
    UndidLoad,
    Written(PathBuf),
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
            slots: [None; 10],
            walking: false,
            cadence: 120,
            muted: false,
            speed: 0.,
            speed_percent: 100,
            turbo_percent: 400,
            turbo: false,
            time: Time::ZERO,
            eeprom: None,
            firmware: String::new(),
            peer: None,
            connection: Connection::Disconnected,
            connection_address: None,
            melon_ds: false,
            saving: false,
            modified: false,
            eeprom_error: None,
            error: None,
            audio_error: None,
            audio_resets: 0,
            audio_buffered: 0,
            audio_rate: 48_000,
            connection_error: None,
            audio_underruns: 0,
            motion_samples: Arc::from([]),
            activity: Arc::from([]),
            display_enabled: false,
            closing: false,
            closed: false,
            completion: None,
            gamepad_press: None,
        }
    }
}

impl Presentation {
    fn completed(&mut self, result: Completion) {
        let next = self
            .completion
            .as_ref()
            .map_or(1, |(id, _)| id.saturating_add(1));
        self.completion = Some((next, result));
    }
    pub fn is_connection_active(&self) -> bool {
        self.peer.is_some() || self.connection.is_active()
    }

    pub fn connection_name(&self) -> String {
        if self.peer.is_some() {
            "Local link".into()
        } else if self.melon_ds && self.connection == Connection::Connected {
            "Connected to melonDS".into()
        } else {
            self.connection.to_string()
        }
    }
}

enum SaveTarget {
    Current,
    Path(PathBuf),
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
    pending_save: Option<SaveTarget>,
    anchor: Instant,
    anchor_time: Time,
    speed_start: (Instant, Time),
    speed: f64,
    emitting: bool,
    sound: Audio,
    samples: Samples,
    audio_resets: u64,
    motion_samples: VecDeque<hs_core::Acceleration>,
    motion_sample: u8,
    activity: VecDeque<InfraredActivity>,
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
            pending_save: None,
            anchor: wall,
            anchor_time: now,
            speed_start: (wall, now),
            speed: 0.,
            emitting: false,
            sound,
            samples,
            audio_resets: 0,
            motion_samples: VecDeque::new(),
            motion_sample: 0,
            activity: VecDeque::new(),
            tcp: None,
        })
    }
    fn reanchor(&mut self) {
        self.anchor = Instant::now();
        self.anchor_time = self.machine.now();
    }
    fn clear_audio(&mut self, rate: u32) -> Result<()> {
        self.audio_resets += 1;
        let mut channel = self.samples.lock().unwrap();
        channel.queue.clear();
        channel.enabled = false;
        channel.primed = false;
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
    fn planned_inputs(&self, end: Time, mut inputs: Vec<TimedInput>) -> Vec<TimedInput> {
        let now = self.machine.now();
        inputs.extend(
            self.inputs
                .iter()
                .take_while(|input| input.at < end)
                .copied(),
        );
        if let Some(at) = self.reset_release.filter(|&at| at < end) {
            inputs.push(TimedInput {
                at: at.max(now),
                input: Input::ResetPin(true),
            });
        }
        let mut next_motion = self.next_motion;
        inputs.extend(self.motion.clone().inputs(&mut next_motion, now, end));
        inputs.sort_by_key(|input| input.at);
        inputs
    }
    fn advance(
        &mut self,
        end: Time,
        mut inputs: Vec<TimedInput>,
        stop_on_ir: bool,
    ) -> Result<Vec<Edge>> {
        let now = self.machine.now();
        if let Some(tcp) = &mut self.tcp {
            inputs.extend(tcp.inputs(now, end, self.anchor, self.anchor_time)?);
        }
        let inputs = self.planned_inputs(end, inputs);
        let mut sink = Sink {
            sound: &mut self.sound,
            samples: Vec::new(),
            edges: Vec::new(),
            dirty: false,
            error: None,
            stop_on_ir,
        };
        self.machine.run_until(end, &inputs, &mut sink)?;
        if let Some(error) = sink.error {
            return Err(error);
        }
        self.dirty |= sink.dirty;
        let Sink { edges, samples, .. } = sink;
        let end = self.machine.now();
        let count = self.inputs.partition_point(|input| input.at < end);
        self.inputs.drain(..count);
        if self.reset_release.is_some_and(|at| at < end) {
            self.reset_release = None;
        }
        for input in self.motion.inputs(&mut self.next_motion, now, end) {
            if let Input::Acceleration(sample) = input.input {
                self.motion_sample = (self.motion_sample + 1) % 4;
                if self.motion_sample == 0 {
                    if self.motion_samples.len() == 512 {
                        self.motion_samples.pop_front();
                    }
                    self.motion_samples.push_back(sample);
                }
            }
        }
        for input in inputs.iter().take_while(|input| input.at < end) {
            if matches!(input.input, Input::InfraredLevel(true)) {
                self.record_activity(input.at, false);
            }
        }
        for edge in &edges {
            if edge.emitting {
                self.record_activity(edge.at, true);
            }
        }
        if let Some(edge) = edges.last() {
            self.emitting = edge.emitting;
        }
        if let Some(tcp) = &mut self.tcp {
            if tcp.connected() {
                for edge in &edges {
                    tcp.emit(edge)?;
                }
                tcp.finish(end)?;
            }
        }
        self.enqueue_audio(samples);
        Ok(edges)
    }
    fn finish_audio(&mut self) -> Result<()> {
        let mut samples = Vec::new();
        self.sound.advance(self.machine.now(), &mut |block| {
            samples.extend_from_slice(block)
        })?;
        self.enqueue_audio(samples);
        Ok(())
    }
    fn enqueue_audio(&self, samples: Vec<i16>) {
        if samples.is_empty() || self.muted || self.fast || self.paused || self.speed_percent != 100
        {
            return;
        }
        let mut channel = self.samples.lock().unwrap();
        channel.enabled = true;
        let available = channel
            .prefill
            .saturating_mul(3)
            .saturating_sub(channel.queue.len());
        channel.queue.extend(samples.into_iter().take(available));
    }
    fn record_activity(&mut self, at: Time, transmit: bool) {
        let at = Time::from_raw((at.raw() >> 59) << 59);
        let index = match self.activity.binary_search_by_key(&at, |sample| sample.at) {
            Ok(index) => index,
            Err(index) => {
                self.activity.insert(
                    index,
                    InfraredActivity {
                        at,
                        sent: 0,
                        received: 0,
                    },
                );
                index
            }
        };
        let sample = &mut self.activity[index];
        let count = if transmit {
            &mut sample.sent
        } else {
            &mut sample.received
        };
        *count = count.saturating_add(1);
        while self
            .activity
            .front()
            .is_some_and(|sample| at.raw().saturating_sub(sample.at.raw()) > (12_u128 << 64))
        {
            self.activity.pop_front();
        }
    }
    fn capture_state(&self) -> PendingSession {
        PendingSession {
            firmware: storage::hash(&self.firmware),
            snapshot: self.machine.snapshot(),
            motion: self.motion.clone(),
            next_motion: self.next_motion,
            reset_release: self.reset_release,
            emitting: self.emitting,
        }
    }
    fn restore(&mut self, saved: PendingSession, rate: u32, linked: bool) -> Result<()> {
        self.machine = Machine::from_snapshot(&saved.snapshot);
        self.motion = saved.motion;
        self.motion_samples.clear();
        self.activity.clear();
        self.next_motion = saved.next_motion;
        self.reset_release = saved.reset_release;
        self.emitting = saved.emitting;
        self.paused = true;
        if linked {
            self.speed_percent = 100;
        }
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
    stop_on_ir: bool,
}
impl Output for Sink<'_> {
    fn exact_stops(&self) -> bool {
        self.stop_on_ir
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
            Event::Infrared { at, emitting } => {
                self.edges.push(Edge { at, emitting });
                if self.stop_on_ir {
                    return ControlFlow::Break(());
                }
            }
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
        let mut audio_check = Instant::now();
        loop {
            let behind = self.sessions.values().any(|s| {
                !s.paused
                    && s.target().raw().saturating_sub(s.machine.now().raw())
                        > hs_core::Duration::from_micros(2000).raw()
            });
            if !behind {
                thread::park_timeout(Duration::from_millis(1));
            }
            for _ in 0..256 {
                match self.receive.try_recv() {
                    Ok((id, command)) => self.dispatch(id, command),
                    Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            while let Ok(reply) = self.replies.try_recv() {
                self.reply(reply);
            }
            if audio_check.elapsed() >= Duration::from_secs(1) {
                if self.sound.needs_restart() {
                    if let Err(error) = self.restart_audio() {
                        *self.sound.error.lock().unwrap() = Some(error.to_string());
                    }
                }
                audio_check = Instant::now();
            }
            self.poll_gamepads();
            if last.elapsed() > Duration::from_millis(500) {
                let linked: Vec<_> = self.links.iter().flat_map(|link| link.ids).collect();
                for (&id, session) in &mut self.sessions {
                    if linked.contains(&id) || session.tcp.as_ref().is_some_and(tcp::Tcp::active) {
                        continue;
                    }
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
    fn restart_audio(&mut self) -> Result<()> {
        let enabled = self.sound.enabled.load(Ordering::Relaxed);
        self.sound = Sound::new();
        self.sound.enabled.store(enabled, Ordering::Relaxed);
        for (&id, session) in &mut self.sessions {
            session.samples = self.sound.add(id);
            session.clear_audio(self.sound.rate)?;
        }
        Ok(())
    }

    fn dispatch(&mut self, id: Id, command: Command) {
        for link in &mut self.links {
            link.prediction = [None, None];
        }
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
            Command::Open { firmware, eeprom } => {
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
                })?;
                return Ok(());
            }
            Command::Sound(enabled) => {
                self.sound.enabled.store(enabled, Ordering::Relaxed);
                return Ok(());
            }
            Command::RetryAudio => return self.restart_audio(),
            Command::GamepadMapping(mapping) => {
                if self.mapping != mapping {
                    self.mapping = mapping;
                    for session in self.sessions.values_mut() {
                        session.release();
                    }
                }
                return Ok(());
            }
            Command::Link(peer) => {
                let error = self
                    .connect(id, peer)
                    .err()
                    .map(|error| format!("{error:#}"));
                self.view(id, |view| view.connection_error = error);
                return Ok(());
            }
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
                self.view(id, |view| {
                    view.error = None;
                    view.completed(Completion::UndidLoad);
                });
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
                self.sessions.remove(&id);
                self.jobs.send(Job::Close(id))?;
                return Ok(());
            }
            Command::PrepareClose => {
                self.disconnect(id)?;
                if let Some(session) = self.sessions.get_mut(&id) {
                    ensure!(
                        !session.machine.eeprom_busy() || !session.paused,
                        "EEPROM programming is paused. Resume before closing."
                    );
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
                }
                return Ok(());
            }
            Command::Pause => {
                let paused = self
                    .sessions
                    .get(&id)
                    .context("Open firmware first")?
                    .paused;
                let ids = if paused {
                    self.links
                        .iter()
                        .find(|link| link.ids.contains(&id))
                        .map(|link| link.ids.to_vec())
                        .unwrap_or_else(|| vec![id])
                } else {
                    self.disconnect(id)?;
                    vec![id]
                };
                for id in ids {
                    let session = self.sessions.get_mut(&id).context("Player closed")?;
                    session.paused = !paused;
                    session.fast = false;
                    session.turbo_toggle = false;
                    session.release();
                    session.reanchor();
                    session.clear_audio(self.sound.rate)?;
                }
                return Ok(());
            }
            Command::Turbo(_)
            | Command::ToggleTurbo
            | Command::Speed(_)
            | Command::TurboRate(_) => {
                let linked = self.links.iter().any(|link| link.ids.contains(&id));
                let session = self.sessions.get_mut(&id).context("Open firmware first")?;
                let connected = linked || session.tcp.as_ref().is_some_and(tcp::Tcp::active);
                let previous = (session.fast, session.speed_percent);
                match command {
                    Command::Turbo(fast) => {
                        ensure!(!fast || !connected, "Connections require 100% speed");
                        session.fast = fast || session.turbo_toggle;
                    }
                    Command::ToggleTurbo => {
                        ensure!(!connected, "Connections require 100% speed");
                        session.turbo_toggle = !session.turbo_toggle;
                        session.fast = session.turbo_toggle;
                    }
                    Command::Speed(percent) => {
                        ensure!(
                            (25..=800).contains(&percent),
                            "Choose a speed between 25% and 800%"
                        );
                        ensure!(
                            percent == 100 || !connected,
                            "Connections require 100% speed"
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
                if previous != (session.fast, session.speed_percent) {
                    session.reanchor();
                    session.clear_audio(self.sound.rate)?;
                }
                return Ok(());
            }
            Command::Reset | Command::Tcp { .. } | Command::MelonDs { .. } => {
                if !matches!(command, Command::Reset) {
                    ensure!(
                        !self
                            .sessions
                            .get(&id)
                            .and_then(|session| session.tcp.as_ref())
                            .is_some_and(tcp::Tcp::active),
                        "Disconnect before starting another connection"
                    );
                }
                if let Command::MelonDs { port } = command {
                    let address = format!("127.0.0.1:{port}");
                    if self.sessions.iter().any(|(&other, session)| {
                        other != id
                            && session.tcp.as_ref().is_some_and(|tcp| {
                                tcp.melon_ds
                                    && tcp.active()
                                    && tcp.address.lock().unwrap().as_ref() == Some(&address)
                            })
                    }) {
                        self.view(id, |view| view.connection_error = Some(
                            "melonDS is connected to another walker. Disconnect it before connecting this walker.".into()
                        ));
                        return Ok(());
                    }
                }
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
            Command::Release => session.release(),
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
                        input: Input::ResetPin(false),
                    }],
                    false,
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
            Command::SaveEepromAs(_) | Command::SaveEeprom => {
                ensure!(
                    !session.saving && session.pending_save.is_none(),
                    "EEPROM is being saved"
                );
                ensure!(
                    !session.machine.eeprom_busy() || !session.paused,
                    "EEPROM programming is paused. Resume before saving."
                );
                session.pending_save = Some(match command {
                    Command::SaveEepromAs(path) => SaveTarget::Path(path),
                    _ => SaveTarget::Current,
                });
                self.view(id, |view| view.eeprom_error = None);
            }
            Command::Tcp {
                address,
                listen,
                allow_remote,
            } => {
                ensure!(!session.paused, "Resume emulation before connecting");
                session.fast = false;
                session.turbo_toggle = false;
                session.speed_percent = 100;
                session.reanchor();
                session.clear_audio(self.sound.rate)?;
                session.tcp = Some(tcp::Tcp::start(
                    address,
                    listen,
                    allow_remote,
                    thread::current(),
                ));
            }
            Command::MelonDs { port } => {
                ensure!(!session.paused, "Resume emulation before connecting");
                ensure!(
                    port != 0,
                    "Choose an ARM9 debugger port between 1 and 65535"
                );
                session.fast = false;
                session.turbo_toggle = false;
                session.speed_percent = 100;
                session.reanchor();
                session.clear_audio(self.sound.rate)?;
                session.tcp = Some(tcp::Tcp::melon_ds(port, thread::current()));
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
            a.tcp.as_ref().is_none_or(|tcp| !tcp.active())
                && b.tcp.as_ref().is_none_or(|tcp| !tcp.active()),
            "Disconnect TCP before linking local players"
        );
        let link = Link::new(
            [id, peer],
            [a.machine.now(), b.machine.now()],
            [a.emitting, b.emitting],
        );
        ensure!(!a.paused && !b.paused, "Resume both players before linking");
        for id in [id, peer] {
            let session = self.sessions.get_mut(&id).unwrap();
            session.tcp = None;
            let changed = session.fast || session.speed_percent != 100;
            session.fast = false;
            session.turbo_toggle = false;
            session.speed_percent = 100;
            session
                .inputs
                .retain(|input| !matches!(input.input, Input::InfraredLevel(_)));
            session.reanchor();
            if changed {
                session.clear_audio(self.sound.rate)?;
            }
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
            self.view(id, |view| view.connection_error = None);
            if let Some(session) = self.sessions.get_mut(&id) {
                if let Some(tcp) = &session.tcp {
                    if tcp.active() {
                        tcp.stop();
                    } else {
                        session.tcp = None;
                    }
                }
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
                    Ok(mut session) => {
                        session.dirty = path.is_none();
                        self.sessions.insert(id, session);
                        self.view(id, |view| {
                            view.ready = true;
                            view.loading = false;
                            view.modified = path.is_none();
                            view.eeprom = path;
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
                }
                self.view(id, |view| {
                    view.completed(Completion::SavedEeprom(path.clone()));
                    view.eeprom = Some(path);
                    view.slots = slots;
                    view.eeprom_error = None;
                });
            }
            Reply::StateWritten { id, path, slots } => self.view(id, |view| {
                view.slots = slots;
                view.error = None;
                view.completed(Completion::SavedState(path));
            }),
            Reply::Written { id, path } => self.view(id, |view| {
                view.error = None;
                view.completed(Completion::Written(path));
            }),
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
                self.view(id, |view| {
                    if view.error.is_none() {
                        view.completed(Completion::LoadedState);
                    }
                });
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
            let target = session.target();
            let end = Time::from_raw(
                session.machine.now().raw() + hs_core::Duration::from_micros(4000).raw(),
            )
            .min(target);
            if end > session.machine.now() {
                if let Err(error) = session.advance(end, Vec::new(), false) {
                    session.paused = true;
                    self.view(id, |view| view.error = Some(format!("{error:#}")));
                }
            }
        }
        for ix in 0..self.links.len() {
            let started = Instant::now();
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
                    let first = usize::from(self.links[ix].sender);
                    let other = 1 - first;
                    if self.links[ix].prediction[first].is_none() {
                        let link = &self.links[ix];
                        let session = &self.sessions[&ids[first]];
                        let horizon = Time::from_raw(link.origins[first].raw() + end.raw());
                        let inputs = session.planned_inputs(horizon, link.inputs(first, end));
                        self.links[ix].prediction[first] = Some(Box::new(optical::predict(
                            &session.machine,
                            horizon,
                            &inputs,
                        )?));
                    }
                    let link = &self.links[ix];
                    let prediction = link.prediction[first].as_ref().unwrap();
                    let horizon = prediction
                        .edges
                        .front()
                        .map_or(prediction.end, |edge| Time::from_raw(edge.at.raw() + 1))
                        .min(prediction.end);
                    let to = Time::from_raw(horizon.raw() - link.origins[first].raw()).min(end);
                    let inputs = link.inputs(other, to);
                    let horizon = Time::from_raw(link.origins[other].raw() + to.raw());
                    let other_edges = self
                        .sessions
                        .get_mut(&ids[other])
                        .unwrap()
                        .advance(horizon, inputs, true)?;
                    let to = Time::from_raw(
                        self.sessions[&ids[other]].machine.now().raw()
                            - self.links[ix].origins[other].raw(),
                    );
                    let inputs = self.links[ix].inputs(first, to);
                    let horizon = Time::from_raw(self.links[ix].origins[first].raw() + to.raw());
                    let first_edges = self
                        .sessions
                        .get_mut(&ids[first])
                        .unwrap()
                        .advance(horizon, inputs, false)?;
                    let link = &mut self.links[ix];
                    for side in [first, other] {
                        link.consume(side, to);
                        if let Some(prediction) = &mut link.prediction[side] {
                            let horizon = Time::from_raw(link.origins[side].raw() + to.raw());
                            while prediction
                                .edges
                                .front()
                                .is_some_and(|edge| edge.at < horizon)
                            {
                                prediction.edges.pop_front();
                            }
                            if prediction.end <= horizon {
                                link.prediction[side] = None;
                            }
                        }
                    }
                    link.transmit(first, first_edges.into_iter())?;
                    link.transmit(other, other_edges.into_iter())?;
                    link.elapsed = to;
                    if started.elapsed() >= Duration::from_millis(4) {
                        break;
                    }
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
        for (&id, session) in &mut self.sessions {
            if let Err(error) = session.finish_audio() {
                session.paused = true;
                if let Some(view) = self.views.get(&id) {
                    view.lock().unwrap().error = Some(error.to_string());
                }
            }
        }
    }
    fn flush(&mut self) {
        let mut stopping = Vec::new();
        for (&id, session) in &mut self.sessions {
            if session.saving || session.machine.eeprom_busy() {
                continue;
            }
            if let Some(path) = session.pending_save.take() {
                let image = session.image();
                let job = match path {
                    SaveTarget::Path(path) => Job::SaveEepromAs { id, path, image },
                    SaveTarget::Current => Job::Commit { id, image },
                };
                if self.jobs.send(job).is_ok() {
                    session.dirty = false;
                    session.saving = true;
                }
            } else if session.closing {
                stopping.push(id);
            }
        }
        for id in stopping {
            let session = self.sessions.get_mut(&id).unwrap();
            if !session.paused {
                session.paused = true;
                session.reanchor();
                let _ = session.clear_audio(self.sound.rate);
            }
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
                view.turbo_percent = session.turbo_percent;
                view.turbo = session.fast;
                view.saving = session.saving || session.pending_save.is_some();
                view.modified = session.dirty || view.saving;
                view.closing = session.closing;
                view.peer = self.links.iter().find(|l| l.ids.contains(&id)).map(|l| {
                    if l.ids[0] == id {
                        l.ids[1]
                    } else {
                        l.ids[0]
                    }
                });
                view.melon_ds = session.tcp.as_ref().is_some_and(|tcp| tcp.melon_ds);
                view.connection = session
                    .tcp
                    .as_ref()
                    .map(|tcp| tcp.state.lock().unwrap().clone())
                    .unwrap_or(Connection::Disconnected);
                view.connection_address = session
                    .tcp
                    .as_ref()
                    .and_then(|tcp| tcp.address.lock().unwrap().clone());
                view.audio_error = self.sound.error.lock().unwrap().clone();
                view.audio_resets = session.audio_resets;
                let channel = session.samples.lock().unwrap();
                view.audio_buffered = channel.queue.len();
                view.audio_rate = self.sound.rate;
                view.audio_underruns = channel.underruns;
                view.motion_samples = session.motion_samples.iter().copied().collect();
                view.activity = session.activity.iter().copied().collect();
                view.display_enabled = session.machine.display_enabled();
            }
        }
    }
    fn poll_gamepads(&mut self) {
        let Some(gamepads) = &mut self.gamepads else {
            return;
        };
        while let Some(event) = gamepads.next_event() {
            for link in &mut self.links {
                link.prediction = [None, None];
            }
            match event.event {
                gilrs::EventType::Disconnected => {
                    for session in self.sessions.values_mut() {
                        session.gamepads.remove(&usize::from(event.id));
                        session.queue_buttons();
                    }
                }
                gilrs::EventType::ButtonPressed(button, _)
                | gilrs::EventType::ButtonReleased(button, _) => {
                    if matches!(event.event, gilrs::EventType::ButtonPressed(..)) {
                        for view in self.views.values() {
                            let mut view = view.lock().unwrap();
                            let next = view.gamepad_press.map_or(1, |(id, _)| id.saturating_add(1));
                            view.gamepad_press = Some((next, button));
                        }
                    }
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
