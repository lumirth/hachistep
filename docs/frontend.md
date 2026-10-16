# Desktop frontend

This example uses `hs-core` through a GPUI Kit player.

On macOS or Windows, build with Rust and the platform's native development tools
installed, including the Windows SDK on Windows:

```sh
cargo run --manifest-path examples/desktop/Cargo.toml -p hs-desktop --release --locked
```

Choose Open Firmware, then open or create save data. Firmware must be a raw
49,152-byte image; save data must contain 65,536 bytes. The application writes
completed guest saves back to the selected EEPROM file. Hardware calibration,
status, and programmed flash live in its adjacent `.hachistep` companion.
Keep that companion with the save. `.hachistep.backup` retains the previous
committed hardware image, and `.hachistep.pending` allows recovery after an
interrupted commit.

A, S, and D operate the physical buttons. Choose Speed in the Emulation menu. Hold Space for temporary turbo. Change
bindings and appearance in Settings. Motion Controls supplies walking;
the guest firmware counts steps. Save states use `.hstepstate` and open paused.
Save State captures both players when locally linked. To restore a linked state,
connect the same two save files in the same order first.

Launch an existing pair of files directly:

```sh
cargo run --manifest-path examples/desktop/Cargo.toml -p hs-desktop --release --locked -- --firmware firmware.bin --save alice.sav
```

After a release build, package the application and its supplied platform icon:

```sh
python3 examples/desktop/package.py --out out/desktop
```

This creates `hachistep.app` on macOS or `hachistep.exe` on Windows.

The example carries AccessKit 0.26.3 with the [KVO teardown fix](https://github.com/AccessKit/accesskit/pull/822) to prevent a macOS crash when closing a window after keyboard input.
