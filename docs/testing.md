# Check hachistep with hachiware

[hachiware](https://github.com/lumirth/hachiware) checks hardware behavior through
guest programs and complete firmware sessions. `tools/hachiware_adapter.py` connects
hachistep to it. Reuse existing coverage; add a check for a bug fix only when it
exposes a genuine behavioral gap.

## Check hardware behavior

Build hachistep in release mode using the [build guide](build.md). From the parent of
that checkout, clone hachiware beside it and run the diagnostics:

```sh
git clone https://github.com/lumirth/hachiware.git
cd hachiware
uv run bench.py run --skip-workloads --out out/diagnostics \
  --target hachistep ../hachistep/tools/hachiware_adapter.py ../hachistep/target/release/hachistep
```

hachiware needs Python 3.10 or newer; `uv run` selects its configured version.
The report groups results by hardware block. For a narrower investigation, the
[diagnostic guide](https://github.com/lumirth/hachiware/blob/main/docs/diagnostics.md)
shows how to select a case and inspect its observations.

## Check a continuation

A checkpoint taken during a save has to preserve the pending write as well as its
current bytes. From the hachiware directory:

```sh
uv run session.py --firmware /path/to/firmware.bin --eeprom /path/to/eeprom.bin \
  --quick --chunk-us 1000 --out out/sessions \
  --target hachistep ../hachistep/tools/hachiware_adapter.py ../hachistep/target/release/hachistep
```

The runner compares state and event history with uninterrupted execution. The
[session guide](https://github.com/lumirth/hachiware/blob/main/docs/sessions.md)
covers image requirements and gameplay checks.

## Measure the affected work

The [benchmark guide](https://github.com/lumirth/hachiware/blob/main/docs/benchmark.md)
compares individual workload speeds with PocketWalker. Keep the whole table when
reporting a result, so a gain in one workload doesn't hide a cost in another. The
behavioral result stays beside each speed. Every command needs a new output directory.
