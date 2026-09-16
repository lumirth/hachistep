# Archive reproduction

A newly created ZIP at code revision
`29f27d330735fffabe44727c97e67b6a9803a101` was extracted into a different
sandbox directory with no target cache. All 840 manifest paths, sizes, hashes and
file modes verified; Git status was clean and `git fsck --full` passed.

From that extraction, all nine `tools/check.py` stages passed: formatting, debug
Rust, release Rust, all-features Rust, Clippy, release build, 17 Python tests,
fixture generation and all 15 guest fixtures. Each Rust configuration passed 110
checks including its doctest. The private test was then enabled separately.

`tools/verify_retail.py --quick --menu-trace` passed with unchanged firmware and
EEPROM identities, complete typed-state/event partition and snapshot replay, and
explicit home/menu regression expectations. Every exported menu byte and all
43,812 product-event records matched the original-directory run exactly.

The final sealing commit adds these records and this description; it does not
change production code, Cargo settings, test expectations or host tools. The
final archive has its own manifest and Git HEAD. It excludes build caches,
compiler distributions, the large input archive, and temporary mutant trees.

These are software reproducibility results, not physical measurements. The
source-only ZIP omits private images/captures; ordinary source/fixture checks
remain runnable without them. See STATUS for hardware fidelity limitations.
