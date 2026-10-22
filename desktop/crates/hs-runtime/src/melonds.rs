// Cartridge contract: ctrl-cheeb-del/pokeheartgold@f63c1d92,
// lib/NitroSDK/src/card/card_spi_wait.c and application/pokewalker/pokewalker_ir.c.
// The adapter replaces only block I/O. Both firmware endpoints own handshakes,
// timeouts, checksums, and persistence; the optical path is shared with TCP.
mod rsp;

use crate::tcp::{Connection, Incoming};
use anyhow::{ensure, Context, Result};
use rsp::{hex, Rsp};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const IMAGE: &[u8; 948] = include_bytes!("melonds/cartridge.bin");
const PARTS: [(usize, usize, &str); 3] = [
    (
        0,
        332,
        "c482f74dba0dd3653d419705614203a13e2808696fb999a2feaec7d04c8e7253",
    ),
    (
        332,
        564,
        "3b85c968fd70e31b5fb24d4705f851b13a8a2283fa622e95fb56d29fd895cac0",
    ),
    (
        564,
        948,
        "592869d9b05fae2251936baad6ef068d7f2e805f01ddc9749759e697860f439f",
    ),
];
const RAM: u32 = 0x0200_0000;
const RX: usize = 144;
const TX: usize = 572;
const MAIL: usize = 484;
const ACTIVE: usize = MAIL + 20;
const SEQUENCE: usize = MAIL + 24;
const EPOCH: usize = 512;
const MAX: usize = 185;
// The write routine has published its complete block and is about to return.
const TRANSMITTED: u32 = 448;

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn mutable(at: usize) -> bool {
    (RX..RX + MAX).contains(&at)
        || (TX..TX + MAX).contains(&at)
        || (MAIL..MAIL + 28).contains(&at)
        || (EPOCH..EPOCH + 16).contains(&at)
}
fn shim_part(bytes: &[u8], start: usize, end: usize) -> bool {
    (start..end).all(|i| mutable(i) || bytes[i] == IMAGE[i])
}
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    for chunk in bytes.as_chunks::<4>().0 {
        let mut instruction = word(chunk, 0);
        // ARM BL relocations and RAM literals vary between languages and builds.
        // Everything else, including branch conditions and peripheral addresses,
        // must match the complete routine, not just its prologue or ROM title.
        if instruction & 0x0f00_0000 == 0x0b00_0000 {
            instruction &= 0xff00_0000;
        } else if (RAM..RAM + 0x40_0000).contains(&instruction) {
            instruction = 0;
        }
        hash.update(instruction.to_le_bytes());
    }
    hex(&hash.finalize())
}
fn recognize(bytes: &[u8]) -> bool {
    PARTS.iter().all(|&(start, end, expected)| {
        shim_part(bytes, start, end) || fingerprint(&bytes[start..end]) == expected
    })
}

fn locate(rsp: &mut Rsp, stop: &AtomicBool) -> Result<u32> {
    let mut ram = Vec::with_capacity(0x40_0000);
    while ram.len() < ram.capacity() {
        ensure!(!stop.load(Ordering::Relaxed), "Connection cancelled");
        ram.extend(rsp.read(RAM + ram.len() as u32, 512)?);
    }
    let candidates: Vec<_> = (0..=ram.len() - IMAGE.len())
        .step_by(4)
        .filter(|&i| {
            let prefix = &ram[i..i + 8];
            (prefix == [0xf0, 0x47, 0x2d, 0xe9, 0x24, 0x21, 0x9f, 0xe5] || prefix == &IMAGE[..8])
                && recognize(&ram[i..i + IMAGE.len()])
        })
        .collect();
    ensure!(
        candidates.len() == 1,
        "Could not identify a supported HGSS cartridge IR driver. Open the game and try again"
    );
    Ok(RAM + candidates[0] as u32)
}

fn install(rsp: &mut Rsp, base: u32) -> Result<()> {
    let original = rsp.read(base, IMAGE.len())?;
    ensure!(recognize(&original), "The game changed while connecting");
    if !shim_part(&original, 0, IMAGE.len()) {
        // CARD_SpiWaitInit marks this before any driver calls. Refuse an active
        // driver: a suspended thread could still hold a return into its body.
        let state = word(&original, 308);
        ensure!(
            (RAM..RAM + 0x40_0000 - 8).contains(&state),
            "Invalid cartridge driver state address"
        );
        ensure!(
            word(&rsp.read(state + 4, 4)?, 0) == 0,
            "Restart the game, then connect HachiStep before opening Connect to Pokéwalker"
        );
        // Each request replaces one whole routine. Write initializes an inactive
        // mailbox; read last publishes the RX buffer. An interrupted
        // install leaves callable routines and never a half-written function.
        for &(start, end, _) in PARTS.iter().rev() {
            rsp.write(base + start as u32, &IMAGE[start..end])?;
        }
        ensure!(
            rsp.read(base, IMAGE.len())? == IMAGE,
            "Could not verify the cartridge connection"
        );
    }
    rsp.write(base + ACTIVE as u32, &0u32.to_le_bytes())?;
    rsp.resume()?;
    // Let any in-flight leaf operation finish with new calls disabled.
    rsp.halt()?;
    ensure!(
        shim_part(&rsp.read(base, IMAGE.len())?, 0, IMAGE.len()),
        "The game changed while connecting"
    );
    rsp.write(base + MAIL as u32, &[0; 24])?;
    rsp.write(base + ACTIVE as u32, &1u32.to_le_bytes())?;
    Ok(())
}

pub(super) fn transport(
    port: u16,
    stop: &AtomicBool,
    state: &Mutex<Connection>,
    outgoing: Receiver<Vec<u8>>,
    incoming: Incoming,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut priority = Priority::new();
    #[cfg(target_os = "macos")]
    priority.set(true);
    #[cfg(target_os = "macos")]
    let _activity = Activity::new();
    let mut rsp = Rsp::connect(port)?;
    let base = locate(&mut rsp, stop)?;
    install(&mut rsp, base)?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_le_bytes();
    let mut sequence = 1u32;
    rsp.write(base + EPOCH as u32, &epoch)?;
    rsp.write(base + SEQUENCE as u32, &sequence.to_le_bytes())?;
    rsp.breakpoint(base + TRANSMITTED)?;
    rsp.resume()?;
    *state.lock().unwrap() = Connection::Connected;
    let mut receive_end = Instant::now();
    let mut heartbeat = Instant::now();
    let mut pending = None;
    let result = (|| -> Result<()> {
        while !stop.load(Ordering::Relaxed) {
            if pending.is_none() {
                pending = outgoing.try_recv().ok();
            }
            if !rsp.wait_stop()? {
                if pending.is_none() && heartbeat.elapsed() < Duration::from_millis(500) {
                    continue;
                }
                rsp.halt()?;
            }
            heartbeat = Instant::now();
            // The live path reads the transmitter and mailbox in one packet.
            // Epoch + sequence reject resets and state restores before any write;
            // the complete adapter is checked when connecting and disconnecting.
            let image = rsp.read(base + MAIL as u32, TX + MAX - MAIL)?;
            ensure!(
                image[EPOCH - MAIL..EPOCH + 16 - MAIL] == epoch
                    && word(&image, SEQUENCE - MAIL) == sequence,
                "melonDS restored or reset the game. Reconnect to continue"
            );
            ensure!(
                image[532 - MAIL..548 - MAIL] == IMAGE[532..548]
                    && image[564 - MAIL..572 - MAIL] == IMAGE[564..572],
                "melonDS changed game or restored a state. Reconnect to continue"
            );
            ensure!(
                word(&image, 16) == 0,
                "Cartridge traffic could not be delivered. Retry the connection"
            );
            let length = word(&image, 8) as usize;
            ensure!(length <= MAX, "Invalid cartridge transmit length");
            let transmitted = length != 0;
            if transmitted {
                let bytes = &image[TX - MAIL..TX - MAIL + length];
                rsp.trace("ds_tx", || hex(bytes));
                let arrived = Instant::now().max(receive_end);
                receive_end = arrived
                    + Duration::from_micros((length as u64 * 10_000_000).div_ceil(115_200) + 1_000);
                for &byte in bytes {
                    incoming
                        .bytes
                        .try_send((arrived, byte))
                        .context("Incoming IR queue is full")?;
                }
                incoming.wake.unpark();
            }
            let mut publication = image[..EPOCH + 16 - MAIL].to_vec();
            publication[8..12].copy_from_slice(&0u32.to_le_bytes());
            let mut received = None;
            if word(&image, 0) == 0 {
                if pending.is_none() && transmitted {
                    // Coalesce a prompt optical reply into this debugger stop.
                    // The walker keeps running at its normal speed throughout.
                    // Bound the wait: cartridge writes can also be one-way.
                    pending = outgoing.recv_timeout(Duration::from_millis(500)).ok();
                }
                if let Some(bytes) = pending.take() {
                    ensure!(
                        (1..=MAX).contains(&bytes.len()),
                        "IR burst exceeds the cartridge buffer"
                    );
                    publication[..4].copy_from_slice(&1u32.to_le_bytes());
                    publication[4..8].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
                    rsp.trace("walker_tx", || hex(&bytes));
                    received = Some(bytes);
                }
            }
            sequence = sequence
                .checked_add(1)
                .context("Connection sequence exhausted")?;
            publication[24..28].copy_from_slice(&sequence.to_le_bytes());
            // CPU execution is stopped: publish RX bytes, TX acknowledgement,
            // and continuation sequence together before resuming the game.
            if let Some(bytes) = received {
                // RX and mailbox span the unchanged write routine. Publishing
                // that same code with the data saves a debugger round trip;
                // ARM9 is stopped and no instruction changes.
                let mut block = IMAGE[RX..EPOCH + 16].to_vec();
                block[..bytes.len()].copy_from_slice(&bytes);
                block[MAIL - RX..].copy_from_slice(&publication);
                rsp.write(base + RX as u32, &block)?;
            } else {
                rsp.write(base + MAIL as u32, &publication)?;
            }
            rsp.resume()?;
        }
        Ok(())
    })();
    // Leave the inert adapter in resident RAM. Resetting the game restores its
    // original driver. No code restore can race a suspended guest return address.
    if rsp.halt().is_ok() {
        if let Ok(image) = rsp.read(base, IMAGE.len()) {
            if shim_part(&image, 0, IMAGE.len()) && image[EPOCH..EPOCH + 16] == epoch {
                let _ = rsp.write(base + ACTIVE as u32, &0u32.to_le_bytes());
            }
        }
    }
    result
}

#[cfg(target_os = "macos")]
// Keep the live link responsive while the user interacts with melonDS. The
// activity and thread priorities end with the link; guest clocks stay unchanged.
struct Activity(
    objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>,
);
#[cfg(target_os = "macos")]
impl Activity {
    fn new() -> Self {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        objc2::rc::autoreleasepool(|_| {
            Self(
                NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                    NSActivityOptions::UserInitiatedAllowingIdleSystemSleep
                        | NSActivityOptions::LatencyCritical,
                    &NSString::from_str("Pokéwalker infrared connection"),
                ),
            )
        })
    }
}
#[cfg(target_os = "macos")]
impl Drop for Activity {
    fn drop(&mut self) {
        objc2::rc::autoreleasepool(|_| unsafe {
            objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.0)
        });
    }
}

#[cfg(target_os = "macos")]
const QOS_USER_INTERACTIVE: u32 = 0x21;
#[cfg(target_os = "macos")]
pub(super) struct Priority {
    original: u32,
    relative: i32,
    active: bool,
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_self() -> *mut std::ffi::c_void;
    fn pthread_get_qos_class_np(
        thread: *mut std::ffi::c_void,
        class: *mut u32,
        relative: *mut i32,
    ) -> i32;
    fn pthread_set_qos_class_self_np(class: u32, priority: i32) -> i32;
}
#[cfg(target_os = "macos")]
impl Priority {
    pub(super) fn new() -> Self {
        let (mut class, mut relative) = (0x15, 0);
        unsafe {
            pthread_get_qos_class_np(pthread_self(), &mut class, &mut relative);
        }
        Self {
            original: if class == 0 { 0x15 } else { class },
            relative,
            active: false,
        }
    }
    pub(super) fn set(&mut self, active: bool) {
        if self.active != active {
            unsafe {
                pthread_set_qos_class_self_np(
                    if active {
                        QOS_USER_INTERACTIVE
                    } else {
                        self.original
                    },
                    if active { 0 } else { self.relative },
                );
            }
            self.active = active;
        }
    }
}
#[cfg(target_os = "macos")]
impl Drop for Priority {
    fn drop(&mut self) {
        self.set(false);
    }
}
