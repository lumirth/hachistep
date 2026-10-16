# Desktop frontend

This example uses `hs-core` through a GPUI Kit player.

On macOS or Windows, build with Rust and the platform's native development tools
installed, including the Windows SDK on Windows:

```sh
cargo run --manifest-path examples/desktop/Cargo.toml -p hs-desktop --release --locked
```

Firmware must be a raw
49,152-byte image; EEPROM images must contain 65,536 bytes. EEPROM changes stay in memory until Save EEPROM or Save EEPROM As. A new EEPROM
starts untitled; closing a modified session offers Save, Discard, or Cancel. Hardware calibration,
status, and programmed flash live in its adjacent `.hachistep` companion.
Keep that companion with the EEPROM. `.hachistep.backup` retains the previous
committed hardware image, and `.hachistep.pending` allows recovery after an
interrupted commit.

State slots 1–10 live in `<eeprom>.states/`. The selected slot is remembered per
EEPROM. State loads open paused and can be undone. Locally linked captures include
both machines and their optical queues. Connect the same two EEPROMs in the same
order before loading a linked state. Untitled EEPROMs can use state files; numbered
slots become available after the EEPROM is saved.

Launch firmware and an EEPROM directly:

```sh
cargo run --manifest-path examples/desktop/Cargo.toml -p hs-desktop --release --locked -- --firmware firmware.bin --eeprom alice.bin
```

After a release build, package the application and its supplied platform icon:

```sh
python3 examples/desktop/package.py --out out/desktop
```

This creates `hachistep.app` and its ZIP on macOS, or `hachistep.exe` on Windows.

The example carries AccessKit 0.26.3 with the [KVO teardown fix](https://github.com/AccessKit/accesskit/pull/822) to prevent a macOS crash when closing a window after keyboard input.
