# Guiding principles

- Emulate the hardware well enough that firmware we haven't seen can work for the same reasons it would on a Pokéwalker. For the purposes of development, retail firmware should be considered just one program it can run.
- Accuracy and speed should improve together. We want one accurate execution path, at least 10 times faster than PocketWalker on every hachiware workload (though that's a general goal, NOT something to optimize for at all costs). We aim to measure realistic use as well as sustained hardware work.
- Keep things succinct and well-organized. LOC bloat can be a useful code smell, and is often the best measure of complexity because most other measures end up just being proxies for LOC. That said, keep useful progress, even when it takes more code. Line count is a reason to question an approach, not a reason to discard an improvement. Be willing to redesign a system when the alternative is more special cases.
- Give frontend authors control over execution and physical inputs. Presentation, transport, and firmware-specific conveniences belong to their applications.
- The same inputs should produce the same device behavior across native and WebAssembly hosts, different execution intervals, and saved-state continuation. Preserve work in progress as well as its eventual result, when it comes to save states.
- Investigate before calling something unknowable. Use component documentation, credible reverse engineering, and `pw`. Record reasoned inferences in the [hardware notes](HARDWARE.md) or beside the mechanism.
- Demonstrate behavior through hachiware's guest programs and complete firmware sessions. Keep the experiments and their expectations there so other emulators can use them too. hachistep owns the core and its adapter.
