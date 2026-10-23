// Cartridge contract: ctrl-cheeb-del/pokeheartgold@f63c1d92,
// lib/NitroSDK/src/card/card_spi_wait.c and src/application/pokewalker/pokewalker_ir.c.
// The adapter replaces block I/O. Both firmware endpoints own the application
// protocol and persistence. The debugger exchanges mailbox data while the game
// runs. Local maintenance covers host stalls without changing either guest clock.
// The optical path is shared with TCP.
mod rsp;

use crate::tcp::{Connection, Incoming};
use anyhow::{ensure, Context, Result};
use rsp::{hex, Rsp};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc, Mutex,
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
const HOST: usize = 128;
const RX: usize = 144;
const TX: usize = 572;
const MAIL: usize = 484;
const ACTIVE: usize = MAIL + 20;
const EPOCH: usize = 512;
const MAX: usize = 136;
const MAINTENANCE: usize = RX + MAX;
pub(crate) type Maintenance = Arc<Mutex<Option<(Instant, [u8; 8])>>>;

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn mutable(at: usize) -> bool {
    (HOST..MAINTENANCE + 8).contains(&at)
        || (TX..TX + MAX * 2).contains(&at)
        || (MAIL..MAIL + 28).contains(&at)
        || (EPOCH..EPOCH + 16).contains(&at)
        || (548..552).contains(&at)
}
fn matches_adapter(bytes: &[u8], start: usize, end: usize) -> bool {
    (start..end).all(|i| mutable(i) || bytes[i] == IMAGE[i])
}
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    for chunk in bytes.as_chunks::<4>().0 {
        let mut instruction = word(chunk, 0);
        // Normalize ARM BL relocations and RAM literals across languages and builds.
        // Match all other bytes, including branch conditions and peripheral addresses.
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
        matches_adapter(bytes, start, end) || fingerprint(&bytes[start..end]) == expected
    })
}

fn locate(rsp: &mut Rsp, stop: &AtomicBool) -> Result<u32> {
    let mut ram = Vec::with_capacity(0x40_0000);
    while ram.len() < ram.capacity() {
        ensure!(!stop.load(Ordering::Relaxed), "Connection canceled");
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
    if candidates.is_empty() {
        ensure!(
            !ram.windows(16).any(|bytes| bytes.starts_with(b"HachiStep IR v") && bytes != &IMAGE[532..548]),
            "This game session contains an older hachistep connection. In melonDS, choose System → Reset, then reconnect before opening Connect to Pokéwalker"
        );
    }
    ensure!(
        candidates.len() == 1,
        "Could not find the Pokéwalker driver. Start HeartGold or SoulSilver and reconnect before opening Connect to Pokéwalker"
    );
    Ok(RAM + candidates[0] as u32)
}

fn install(rsp: &mut Rsp, base: u32) -> Result<()> {
    let original = rsp.read(base, IMAGE.len())?;
    ensure!(recognize(&original), "The game changed while connecting");
    if !matches_adapter(&original, 0, IMAGE.len()) {
        // CARD_SpiWaitInit marks this before any driver calls. Refuse an active
        // driver because a suspended thread could hold a return address inside it.
        let state = word(&original, 308);
        ensure!(
            (RAM..RAM + 0x40_0000 - 8).contains(&state),
            "Invalid cartridge driver state address"
        );
        ensure!(
            word(&rsp.read(state + 4, 4)?, 0) == 0,
            "Restart the game, then connect hachistep before opening Connect to Pokéwalker"
        );
        // Replace whole routines, with read last, so an interrupted installation
        // leaves callable functions and an inactive mailbox.
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
        matches_adapter(&rsp.read(base, IMAGE.len())?, 0, IMAGE.len()),
        "The game changed while connecting"
    );
    // Clear old session data, including its maintenance token, before activation.
    rsp.write(base + HOST as u32, &[0; MAINTENANCE + 8 - HOST])?;
    rsp.write(base + MAIL as u32, &[0; 28])?;
    rsp.write(base + ACTIVE as u32, &1u32.to_le_bytes())?;
    Ok(())
}

// pw src/support/ir.c and HGSS pokewalker_ir.c ignore command 0x2E but
// renew the receive timer. Learn its session token from a checksum-valid
// application frame. Commands 0x24 and 0x26 would advance the transfer.
fn maintenance(bytes: &[u8], current: &mut Option<[u8; 8]>, closed: &mut bool) {
    if bytes == [0xfc ^ 0xaa] {
        *current = None;
        *closed = false;
        return;
    }
    if bytes.len() < 8 {
        return;
    }
    let mut decoded: Vec<_> = bytes.iter().map(|b| b ^ 0xaa).collect();
    let expected = u16::from_le_bytes([decoded[2], decoded[3]]);
    decoded[2..4].fill(0);
    if checksum(&decoded) != expected {
        return;
    }
    if decoded[0] == 0xf4 {
        *current = None;
        *closed = true;
    } else if !*closed && decoded[0] < 0xf0 {
        let mut frame = [0u8; 8];
        frame[0] = 0x2e;
        frame[4..].copy_from_slice(&decoded[4..8]);
        let sum = checksum(&frame);
        frame[2..4].copy_from_slice(&sum.to_le_bytes());
        *current = Some(frame);
    }
}
fn checksum(bytes: &[u8]) -> u16 {
    let sum = bytes.iter().enumerate().fold(2u32, |sum, (i, b)| {
        sum + (u32::from(*b) << if i % 2 == 0 { 8 } else { 0 })
    });
    let sum = (sum & 0xffff) + (sum >> 16);
    (sum + (sum >> 16)) as u16
}

pub(super) fn transport(
    port: u16,
    stop: &AtomicBool,
    state: &Mutex<Connection>,
    outgoing: Receiver<Vec<u8>>,
    incoming: Incoming,
) -> Result<()> {
    let mut rsp = Rsp::connect(port)?;
    {
        let mut state = state.lock().unwrap();
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        *state = Connection::Preparing;
    }
    let base = locate(&mut rsp, stop)?;
    install(&mut rsp, base)?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_le_bytes();
    rsp.write(base + EPOCH as u32, &epoch)?;
    rsp.resume()?;
    {
        let mut state = state.lock().unwrap();
        if !stop.load(Ordering::Relaxed) {
            *state = Connection::Connected;
        }
    }
    let mut receive_end = Instant::now();
    let mut rx_sequence = 0u32;
    let mut tx_sequence = 0u32;
    let mut rx_consumed = 0u32;
    let mut pending = None;
    let mut upkeep = None;
    let mut closed = false;
    let mut heartbeat = Instant::now();
    let mut progress = Instant::now();
    let mut host_sequence = 0u32;
    let mut block = [0; MAINTENANCE + 8 - HOST];
    let result = (|| -> Result<()> {
        while !stop.load(Ordering::Relaxed) {
            // melonDS 1.1 services GDB requests on the emulation thread at frame
            // boundaries. Guest execution cannot interleave a memory request.
            let image = rsp.read(base + MAIL as u32, TX + MAX * 2 - MAIL)?;
            ensure!(
                image[EPOCH - MAIL..EPOCH + 16 - MAIL] == epoch
                    && image[532 - MAIL..548 - MAIL] == IMAGE[532..548]
                    && image[564 - MAIL..572 - MAIL] == IMAGE[564..572],
                "melonDS changed game or restored a state. Reconnect to continue"
            );
            let consumed = word(&image, 0);
            ensure!(
                consumed == rx_consumed || consumed == rx_sequence,
                "melonDS restored the game. Reconnect to continue"
            );
            rx_consumed = consumed;
            ensure!(
                word(&image, 16) == 0,
                "Cartridge traffic could not be delivered. Retry the connection"
            );
            let transmitted = word(&image, 4);
            let new_transmission = transmitted != tx_sequence;
            if new_transmission {
                let count = transmitted.wrapping_sub(tx_sequence);
                ensure!(
                    count <= 2,
                    "melonDS restored the game. Reconnect to continue"
                );
                for _ in 0..count {
                    let slot = (tx_sequence & 1) as usize;
                    let length = word(&image, 8 + slot * 4) as usize;
                    ensure!(
                        (1..=MAX).contains(&length),
                        "Invalid cartridge transmit length"
                    );
                    let offset = TX - MAIL + slot * MAX;
                    let bytes = &image[offset..offset + length];
                    maintenance(bytes, &mut upkeep, &mut closed);
                    rsp.trace("ds_tx", || hex(bytes));
                    let arrived = Instant::now().max(receive_end);
                    receive_end = arrived
                        + Duration::from_micros(
                            (length as u64 * 10_000_000).div_ceil(115_200) + 1_000,
                        );
                    for &byte in bytes {
                        incoming
                            .bytes
                            .try_send((arrived, byte))
                            .context("Incoming IR queue is full")?;
                    }
                    incoming.wake.unpark();
                    tx_sequence = tx_sequence
                        .checked_add(1)
                        .context("Connection sequence exhausted")?;
                }
            }
            if pending.is_none() {
                pending = outgoing.try_recv().ok();
                if pending.is_none() && new_transmission && consumed == rx_sequence {
                    // Let a prompt optical reply share the TX acknowledgement.
                    // Otherwise an idle snapshot can occupy a whole DS frame
                    // ahead of a response already being emitted by the walker.
                    pending = outgoing.recv_timeout(Duration::from_millis(25)).ok();
                }
            }
            block[..4].copy_from_slice(&tx_sequence.to_le_bytes());
            let mut received = false;
            if consumed == rx_sequence {
                if let Some(bytes) = pending.take() {
                    ensure!(
                        (1..=MAX).contains(&bytes.len()),
                        "IR burst exceeds the cartridge buffer"
                    );
                    rx_sequence = rx_sequence
                        .checked_add(1)
                        .context("Connection sequence exhausted")?;
                    block[4..8].copy_from_slice(&rx_sequence.to_le_bytes());
                    block[8..12].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
                    maintenance(&bytes, &mut upkeep, &mut closed);
                    rsp.trace("walker_tx", || hex(&bytes));
                    block[RX - HOST..RX - HOST + bytes.len()].copy_from_slice(&bytes);
                    received = true;
                }
            }
            if new_transmission || received {
                progress = Instant::now();
            }
            // Maintenance cannot conceal a stalled application indefinitely.
            let live = upkeep.filter(|_| progress.elapsed() < Duration::from_secs(3));
            *incoming.maintenance.as_ref().unwrap().lock().unwrap() =
                live.map(|frame| (Instant::now(), frame.map(|b| b ^ 0xaa)));
            block[MAINTENANCE - HOST..].copy_from_slice(&live.unwrap_or([0; 8]));
            if new_transmission || received || heartbeat.elapsed() >= Duration::from_millis(250) {
                host_sequence = host_sequence
                    .checked_add(1)
                    .context("Connection sequence exhausted")?;
                block[12..16].copy_from_slice(&host_sequence.to_le_bytes());
                heartbeat = Instant::now();
                // Write host-owned fields only. A TX acknowledgement must not
                // overwrite the guest counters or a newly published packet.
                rsp.write(base + HOST as u32, &block)?;
            }
        }
        Ok(())
    })();
    *incoming.maintenance.as_ref().unwrap().lock().unwrap() = None;
    // Disable the adapter and leave its code in RAM. Restoring the original
    // driver could invalidate a suspended thread's return address.
    if rsp.halt().is_ok() {
        if let Ok(image) = rsp.read(base, IMAGE.len()) {
            if matches_adapter(&image, 0, IMAGE.len()) && image[EPOCH..EPOCH + 16] == epoch {
                let _ = rsp.write(base + ACTIVE as u32, &0u32.to_le_bytes());
            }
        }
    }
    result
}
