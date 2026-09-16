# Revision evidence index

Original starter evidence remains in the parent directory. This directory records
incremental development and the final v0.2 gates separately. No physical captures.

- `baseline-tests.log`: original-tree check at the start of this continuation.
- `*-tests.log`, `retail-after-*.log`, and `workspace-timer-w.log`: incremental checks.
- `initial-check-*`, `initial-conformance.json`: first complete v0.2 gate run.
- `mutations-initial/`: three control/pass, compiled-mutant/assertion-fail pairs.
- `initial-workloads/`: all four unmodified private-input observations, before
  the verifier gained explicit reviewed software expectations.
- `initial-aec-benchmark/`: eight timed samples per binary in four ABBA/BAAB
  blocks. Baseline 7361f1c, optimized source 06d83a2. Not an original-starter
  comparison. Reports do not contain ROM/EEPROM/image bytes.
- `aec-optimization-equivalence.json`: all six exported byte images and all
  43,812 menu product-event records match across that isolated optimization.
- `release-check/`, `release-retail/`, `release-mutations/`, `release-benchmark/`:
  final repeated gates when present; summaries record exact source state.

A source marked dirty records its changed/untracked paths honestly. The archive's
manifest and Git history identify the delivered tree; do not infer a dirty run
was a different unrecorded clean commit. Later sealing commits only add evidence
or describe extraction, unless their Git diff explicitly shows implementation.

Private pictures/audio are in ignored `private-observations`, not this directory.
Software-observed baselines do not become physical truth by passing regression.
