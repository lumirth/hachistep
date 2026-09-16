# Recorded build and execution evidence

These are observed runs of this starter, not physical-hardware captures.
`build-checks.json`, `retail.json` and `benchmark.json` identify the clean
source revision, commands and actual compiler. Later commits may add evidence
and documentation without altering the tested Rust code.

- `tests.log`: 47 ordinary Rust tests plus one doctest; private case ignored.
- `release-tests.log`: same corpus in release mode.
- `trace-tests.log`: same corpus with bus observation compiled in.
- `retail-replay.log`: the opted-in complete event/state and snapshot test.
- `conformance.json`: seven independent software diagnostic cases.
- `python-tests.log`: five host-tool tests.
- `home/menu/walking/idle.json`: actual endpoint and I/O observations.
- `benchmark.json`: raw repeated measurements, not a fastest-emulator claim.
- `input-provenance.json`: exact images and compiler artifact identity.

The private image viewer/WAV are outside Git in `private-observations/`.
Source inputs remained unchanged. Reports may contain sandbox-local paths,
which identify the recorded invocation rather than a required installation path.
