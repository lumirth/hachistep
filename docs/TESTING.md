# Behavioral validation

[Hachiware](https://github.com/lumirth/hachiware) owns hardware diagnostics, firmware
sessions and emulator comparisons. HachiStep supplies its CLI and adapter.

Build and check HachiStep:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --release --locked
```

From a Hachiware checkout, run the guest diagnostics:

```sh
uv run bench.py run --skip-workloads --out out/check-1 \
  --target hachistep ../hachistep/tools/hachiware_adapter.py ../hachistep/target/release/hachistep
```

Run supplied firmware through complete sessions, including save-state continuation
and different host-call horizons:

```sh
uv run session.py --firmware firmware.bin --eeprom eeprom.bin --quick --chunk-us 1000 \
  --out out/sessions-1 \
  --target hachistep ../hachistep/tools/hachiware_adapter.py ../hachistep/target/release/hachistep
```

The [session guide](https://github.com/lumirth/hachiware/blob/main/docs/sessions.md)
covers gameplay, peer exchange and comparisons between builds. The
[benchmark guide](https://github.com/lumirth/hachiware/blob/main/docs/benchmark.md)
covers per-workload conformance and speed comparisons, including PocketWalker.

Choose checks for the affected behavior and reuse results until a change or unresolved
failure warrants another run. Hardware expectations come from independent evidence;
reproducibility checks establish agreement between executions. Keep individual results
in the task, PR or issue.
