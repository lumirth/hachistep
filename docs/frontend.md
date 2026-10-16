# Desktop frontend

A GPUI Kit frontend for `hs-core`.

On macOS or Windows, build with Rust and the platform's native development tools
installed, including the Windows SDK on Windows:

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

Launch firmware and an EEPROM directly:

```sh
cargo run --manifest-path desktop/Cargo.toml -p hs-desktop --release --locked -- --firmware firmware.bin --eeprom alice.bin
```

After a release build, package the application and its supplied platform icon:

```sh
python3 desktop/package.py --out out/desktop
```

This creates `hachistep.app` and its ZIP on macOS, or `hachistep.exe` on Windows.

The desktop pins an AccessKit 0.26.3 [KVO teardown backport](https://github.com/lumirth/accesskit/commit/e09cf84b8e0af7659423e5b91eed41c5ac2dd693) to prevent a macOS crash when closing a window after keyboard input. [Issue #10](https://github.com/lumirth/hachistep/issues/10) tracks returning to a published release.
