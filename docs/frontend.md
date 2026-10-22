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

To connect a walker to stock melonDS 1.1 on the same computer:

1. In melonDS, enable the ARM9 GDB stub under **Config → Emu settings → Devtools**.
   The debugger accepts unauthenticated connections from your network.
2. Under **CPU emulation**, clear **Enable JIT recompiler**. Restart melonDS.
3. Start HeartGold or SoulSilver in melonDS.
4. In HachiStep, choose **Tools → Infrared → Connect to melonDS…**.
   If you changed the ARM9 port in melonDS, enter it under **ARM9 debugger port**.
   Click **Connect** and wait until HachiStep connects.
5. On the game's main menu, select **Connect to Pokéwalker**.
   Follow the prompts until the game waits for the walker.
6. On a new walker's home screen, press the middle button. On a registered walker,
   select **Connect**. If the display is asleep, wake the walker and release the
   button first.

Keep both emulators at normal speed during transfers. Disconnect HachiStep before
changing games or loading a melonDS state, then reconnect. When you finish using
the connection, disable the GDB stub.

HachiStep recognizes the cartridge IR driver across all HGSS languages and in
hacks that retain the retail driver and the standard Pokéwalker protocol. It rejects
unrecognized drivers. The connection installs a temporary adapter in game RAM.
Reset the game to remove the adapter.

Launch firmware and an EEPROM directly:

```sh
cargo run --manifest-path desktop/Cargo.toml -p hs-desktop --release --locked -- --firmware firmware.bin --eeprom alice.bin
```

Packaging requires Python 3.11 or newer and a release desktop build. On macOS,
install the DMG builder in a virtual environment first:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r desktop/requirements-macos.txt
.venv/bin/python desktop/package.py --out out/desktop
```

On Windows and Linux, run `python3 desktop/package.py --out out/desktop`.
The downloads are a macOS DMG, a Windows ZIP, and a Linux tarball. To install on
macOS, open the DMG and drag hachistep to Applications. Linux needs an X11 or
Wayland session and a Vulkan driver.

The Release workflow builds downloads for version tags. Run it manually to get
review downloads for one platform or all platforms. CI builds packages when
packaging inputs change.

The desktop pins an AccessKit 0.26.3 [KVO teardown backport](https://github.com/lumirth/accesskit/commit/e09cf84b8e0af7659423e5b91eed41c5ac2dd693) to prevent a macOS crash when closing a window after keyboard input. [Issue #10](https://github.com/lumirth/hachistep/issues/10) tracks returning to a published release.
