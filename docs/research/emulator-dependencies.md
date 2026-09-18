# Emulator core dependencies and Rust `std`

Research checked 2026-09-16. These four handheld emulators show how existing projects
divide dependencies among the core, frontend, build tools and tests.

## Observed practice

| Project | Core and runtime | Frontend, build, and test distinction |
| --- | --- | --- |
| SameBoy | Its `lib` target builds the `Core` sources separately. The implementation uses the C standard library and allocates the machine, RAM, VRAM, and ROM. This is a reusable core, not a freestanding or allocation-free program. | SDL/OpenGL belong to the SDL frontend link. The core library and tester are separate targets; RGBDS is a build tool for boot ROMs. [Build targets](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Makefile#L398-L450), [frontend link](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Makefile#L678-L733), [allocation](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.c#L157-L175). |
| binjgb | The same C emulator source is included in desktop, headless tester, and browser builds. It uses standard C facilities and allocates machine/audio storage. | SDL/OpenGL are linked to desktop frontends; the tester and Emscripten target have their own source lists without those desktop libraries. ImGui is debugger UI code. [Target definitions](https://github.com/binji/binjgb/blob/c60e138da5a795ebb55e56b11b7e90024e41112c/CMakeLists.txt#L38-L150), [core allocation](https://github.com/binji/binjgb/blob/c60e138da5a795ebb55e56b11b7e90024e41112c/src/emulator.c#L5094-L5111). |
| mGBA | The library has optional utility features such as zlib, PNG, SQLite, and scripting. Even the core source set includes vendored third-party code (`inih`), so few external system packages does not mean zero reused code. | Qt and SDL frontends are independently selectable. The cmocka test dependency is tied to `BUILD_SUITE`. Utility dependencies enabled in the library are real library dependencies, even when the UI is absent. [Feature configuration](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L54-L72), [test dependency](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L499-L521), [vendored source](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L109-L110), [core composition](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L917-L988). |
| jgenesis Game Boy core | This Rust core uses `std`. Its manifest depends on `bincode` with derive support, `log`, `rand`, `thiserror`, and internal common/config/DSP/macro crates. Its API receives rendering, audio, input, and save services through frontend traits. | GPU rendering, SDL3 host integration, GUI, and WebAssembly frontend live in separate crates. The project demonstrates that a Rust `std` core with selected generic dependencies can retain distinct native and browser frontends. [Exact core manifest](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/Cargo.toml), [use of std](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/src/lib.rs#L17-L29), [embedding API](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/src/api.rs#L211-L234), [architecture](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/ARCHITECTURE.md#L126-L150). |

The shared lesson is to keep host integration out of the hardware engine and make
dependency costs visible. Their dependency counts differ. The C examples establish those
architectural boundaries; C use of libc does not directly settle Rust's `std` versus
`no_std` choice.

## Three independent choices

- `no_std` changes Rust's implicit standard-library linkage and prelude.
  It does not prohibit third-party crates, and dependencies can still link
  `std`. [Rust Reference](https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-attribute)
- No third-party runtime dependencies concerns the dependency graph. It does
  not imply no standard library, no allocation, or no generated code.
- No allocation during ordinary execution concerns which operations the
  running core performs. A `std` core can satisfy it; a `no_std` core can allocate
  through `alloc`, which provides `Box`, `Vec`, and other heap-backed facilities.
  [Rust alloc documentation](https://doc.rust-lang.org/alloc/)

Browser support does not require `no_std`. Rust's `wasm32-unknown-unknown` target
supports `std`, `core`, and `alloc`; OS-dependent functions such as filesystem access
and thread creation are unavailable or fail. Keeping those services in the frontend is
the relevant boundary. ([Rust target
documentation](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html))

Build tools and procedural macros run during compilation; development dependencies
support tests/examples/benchmarks rather than ordinary downstream use. They still have
build-time and maintenance costs, while generated code can have runtime costs. Count
those separately instead of presenting one dependency total as a performance result.
([Cargo dependency
kinds](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#development-dependencies),
[procedural macros](https://doc.rust-lang.org/reference/procedural-macros.html))

## HachiStep dependency rationale

The [design](../DESIGN.md#36-rust-implementation-policy) defines the policy: `std`, few
justified dependencies, allocation-free ordinary execution, and safe Rust by default
with narrowly justified exceptions. Safe dependency APIs do not require their internals
to be free of unsafe code. Manifests and `Cargo.lock` own versions and features;
[BUILD](../BUILD.md) owns the compiler requirements.

| Dependency | Location and reason |
| --- | --- |
| Borsh | Core save state encoding. Fixed-width integer rules and support for the actual fixed-array records avoid a handwritten paired encoder/decoder. Use `std` and `derive`, without schema-generation machinery. |
| SHA-2 | Core firmware identity/state checksum and CLI image identities. Replaces custom digest code; the selected `force-soft` backend keeps hashing portable. It does no work in ordinary emulation. |
| Serde JSON | CLI report encoding, including full-width time values. Keeps escaping and serialization out of handwritten format strings. |
| Clap | CLI option declarations, relationships, validation, and generated help share one definition. |
| Proptest | Development-only generation and shrinking of execution partitions and capture points. It supplies inputs, not hardware expectations. |

Borsh is language-independent but not self-describing. Its byte rules do not establish
what a field means or whether another emulator can restore it. HachiStep first defines
retained hardware work, completed effects, and clock obligations, then encodes that
contract. Derives are appropriate where the types already express it; compact executor
continuations are mapped during save/load. Large-array support is useful integration
support, not the entire justification. See the [native state
contract](../SAVE_STATES.md) and
[Borsh specification](https://github.com/near/borsh#specification).

The codec comparison considered Postcard/Serde and Bitcode as viable alternatives.
Postcard's variable-length integers and extra fixed-array adapters brought no needed
capability for this contract. Bitcode's packing added mechanisms without a demonstrated
capture/size benefit for this workload. Borsh's explicit encoding and shared field
definitions fit the chosen native records.
[Postcard wire format](https://postcard.jamesmunns.com/wire-format.html),
[Serde array adapter](https://docs.rs/serde-big-array/0.5.1/serde_big_array/),
[Bitcode implementation](https://docs.rs/bitcode/0.6.9/bitcode/).

Adopt future dependencies for work they remove. Choose parsing tools when the CPU
generator's source format is defined. Error formatting is small enough to keep explicit.
