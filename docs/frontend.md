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
The first firmware you open becomes the default. It is copied into application
storage and used by new windows. Change it in **Settings → General**.
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
EEPROM. Loading a state pauses emulation and can be undone. Locally linked captures include
both machines and their optical queues. Connect the same two EEPROMs in the same
order before loading a linked state. Untitled EEPROMs can use state files; numbered
slots become available after the EEPROM is saved.

The **System** menu controls playback, walking, and state slots. F1–F10 load slots;
Shift+F1 through Shift+F10 save them. **Quick Slot** chooses the slot for Quick Save and Quick Load.
**Settings → Hotkeys** changes emulator shortcuts and held controls.

Palette, LCD contrast, status bar, speed, fast-forward rate, walking pace, and mute
are remembered per EEPROM. **Settings → General** sets playback defaults for new
windows. Hold W to walk, or enable **System → Walk**.

**Settings → Display** manages palettes, including the editable included palettes.
Its **More** menu imports palettes, copies one or all, and restores the included
set. Import and Copy All use one `Name|FFFFFF|AAAAAA|555555|000000` record per line.
The four colors run from background to foreground.

**View → Apply LCD Contrast** applies the firmware's contrast setting. Turn it off
to see the palette colors directly. The palette editor offers the same choice for
its live preview.

Infrared connections require 100% speed. Pausing, resetting, or closing a player
disconnects it. Local links preserve each machine's clock state and forward optical
transitions at the next 64.64 time unit after emission. Snapshot lookahead finds the
next transition before either recipient runs past it. TCP exchanges decoded
115200-baud 8N1 bytes, groups outgoing optical bursts into socket writes, and
reconstructs optical input at observed arrival times. The wire format carries no
sender timestamps.

To connect a walker to stock melonDS 1.1 on the same computer, set up melonDS once:

1. Enable the ARM9 GDB stub under **Config → Emu settings → Devtools**.
   The debugger accepts unauthenticated connections from your network.
2. Under **CPU emulation**, clear **Enable JIT recompiler** and restart melonDS.

For each session:

1. Start HeartGold or SoulSilver in melonDS.
2. In HachiStep, choose **Tools → Infrared…** and select **melonDS**.
   If you changed the ARM9 port, enter it beside **ARM9 port**.
   Click **Connect** and wait until HachiStep connects.
3. On the game's main menu, select **Connect to Pokéwalker**.
   Follow the prompts until the game waits for the walker.
4. On a new walker's home screen, press the middle button. On a registered walker,
   select **Connect**. If the display is asleep, wake the walker and release the
   button first. In the Infrared window, **Show Walker** focuses the walker.
   S is the default middle key.

The Infrared window also offers **Local Link** for other open walkers and **TCP**
for compatible emulators. The connection diagram shows optical activity in each
direction. The game and walker display the transfer result.

Keep both emulators at normal speed during transfers. Disconnect HachiStep before
changing games or loading a melonDS state, then reconnect.
Only one walker can use a melonDS debugger port at a time. If a game session contains
an older HachiStep adapter, reset the game in melonDS before reconnecting.

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
