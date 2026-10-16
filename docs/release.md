The first desktop release of hachistep. The project is in alpha development.

- LCD player with native menus, keyboard and gamepad controls, and configurable palettes and margins.
- Walking simulation, local infrared linking, and TCP connections to compatible emulators.
- Ten save-state slots, state files, and undo for state loads.
- EEPROM changes stay in memory until saved. Firmware can be retained for new windows.
- LCD writes stop at the end of display RAM.

Desktop downloads include the CLI as `hachistep-cli`. macOS downloads are available for Apple silicon and Intel; Windows and Linux downloads are x86-64. Linux builds target Ubuntu 24.04 and compatible systems. The WASI download contains the CLI for Wasmtime.

Core, CLI, runtime, and desktop share version 0.3.0 and the `v0.3.0` source tag.
