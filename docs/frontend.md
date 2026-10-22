# Desktop frontend

A GPUI Kit frontend for `hs-core`.

Build with Rust and the platform's native development tools, including the
Windows SDK on Windows. On Ubuntu 24.04, install the native libraries first:

```sh
sudo apt install clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev libudev-dev libvulkan1
```

Then build:

```sh
cargo run --manifest-path desktop/Cargo.toml -p hs-desktop --release --locked
```

Firmware must be a raw 49,152-byte image; EEPROM images must contain 65,536 bytes.
A default firmware is copied into application storage and used by new windows.
Existing sessions retain their firmware. Recent EEPROMs remember their firmware's
SHA-256 identity; a missing image is resolved from retained copies or located again.

EEPROM changes stay in memory until Save EEPROM or Save EEPROM As. A new EEPROM
starts untitled; closing a modified session offers Save, Don't Save, or Cancel.
Hardware calibration, status, and programmed flash live in its adjacent `.hachistep`
companion.
Keep that companion with the EEPROM. `.hachistep.backup` retains the previous
committed hardware image, and `.hachistep.pending` allows recovery after an
interrupted commit.

State slots 1–10 live in `<eeprom>.states/`. The selected slot is remembered per
EEPROM. State loads open paused and can be undone. Locally linked captures include
both machines and their optical queues. Connect the same two EEPROMs in the same
order before loading a linked state. Untitled EEPROMs can use state files; numbered
slots become available after the EEPROM is saved.

Infrared connections require 100% speed. Pausing, resetting, or closing a player
disconnects it. Local links preserve each machine's clock state and forward optical
transitions at the next 64.64 time unit after emission. Snapshot lookahead finds the
next transition before either recipient runs past it. TCP exchanges decoded
115200-baud 8N1 bytes, groups outgoing optical bursts into socket writes, and
reconstructs optical input at observed arrival times. The wire format carries no
sender timestamps.

Use **Tools → Infrared → Connect to melonDS…** with stock melonDS 1.1 on the
same computer. In melonDS, enable the ARM9 GDB stub under **Config → Emu settings
→ Devtools**, use the interpreter, and restart. Open HeartGold or SoulSilver,
connect HachiStep, then enter the game's Pokéwalker menu. Finish the game's
instructions before connecting from the walker. On a new walker's home screen,
press the middle button; on a registered walker, select Connect. If the display
is asleep, wake it first, then release the button before connecting. Keep both
emulators at normal speed during a transfer. Disable the GDB stub afterward:
melonDS exposes its unauthenticated debugger to the network, even though HachiStep
connects locally.

The connection recognizes the cartridge IR routines in RAM across all HGSS
languages, including relocated routines in hacks that retain that driver. An
unrecognized driver is rejected. It installs a temporary cartridge adapter in
those routines; game and walker firmware handle the protocol. ROM files and
the melonDS executable stay untouched. Reset the game to remove the adapter.
Disconnect before changing games or loading a state, then reconnect. Connect
before entering the Pokéwalker menu on a fresh game boot so installation cannot
replace an active
cartridge operation.

After installation, the game runs continuously while the debugger exchanges
mailbox data. Each endpoint receives local `0x2E` maintenance frames during an
active transfer, so debugger latency does not consume its short inactivity
window. Genuine packets take priority; maintenance ends on shutdown, lost host
contact, or three seconds without application traffic. The walker receives
ordinary optical input, with its core execution and pacing unchanged.

Launch firmware and an EEPROM directly:

```sh
cargo run --manifest-path desktop/Cargo.toml -p hs-desktop --release --locked -- --firmware firmware.bin --eeprom alice.bin
```

Core, CLI, runtime, and desktop use one release version. Build the CLI too before
packaging the desktop download. Packaging requires Python 3.11 or newer:

```sh
cargo build --release --locked -p hs-cli
python3 desktop/package.py --out out/desktop
```

The versioned ZIP or Linux tarball contains the desktop application and
`hachistep-cli`. Linux needs an X11 or Wayland session and a Vulkan driver.

The Release workflow builds downloads for version tags. Run it manually to get
review archives for one platform or all platforms. CI checks development builds
on ordinary changes and packaged builds when packaging inputs change.

The desktop pins an AccessKit 0.26.3 [KVO teardown backport](https://github.com/lumirth/accesskit/commit/e09cf84b8e0af7659423e5b91eed41c5ac2dd693) to prevent a macOS crash when closing a window after keyboard input. [Issue #10](https://github.com/lumirth/hachistep/issues/10) tracks returning to a published release.
