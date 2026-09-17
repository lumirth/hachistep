# Emulator core dependencies and Rust `std`

Research checked 2026-09-16. These four handheld-emulator implementations offer
useful examples of dependency boundaries; they are not a ranking or a claim that
HachiStep should copy their whole architectures.

## Observed practice

| Project | Core and runtime | Frontend, build, and test distinction |
| --- | --- | --- |
| **SameBoy** | Its `lib` target builds the `Core` sources separately. The implementation uses the C standard library and allocates the machine, RAM, VRAM, and ROM. This is a reusable core, not a freestanding or allocation-free program. | SDL/OpenGL belong to the SDL frontend link. The core library and tester are separate targets; RGBDS is a build tool for boot ROMs. [Build targets](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Makefile#L398-L450), [frontend link](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Makefile#L678-L733), [allocation](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.c#L157-L175). |
| **binjgb** | The same C emulator source is included in desktop, headless tester, and browser builds. It uses standard C facilities and allocates machine/audio storage. | SDL/OpenGL are linked to desktop frontends; the tester and Emscripten target have their own source lists without those desktop libraries. ImGui is debugger UI code. [Target definitions](https://github.com/binji/binjgb/blob/c60e138da5a795ebb55e56b11b7e90024e41112c/CMakeLists.txt#L38-L150), [core allocation](https://github.com/binji/binjgb/blob/c60e138da5a795ebb55e56b11b7e90024e41112c/src/emulator.c#L5094-L5111). |
| **mGBA** | The library has optional utility features such as zlib, PNG, SQLite, and scripting. Even the core source set includes vendored third-party code (`inih`), so few external system packages does not mean zero reused code. | Qt and SDL frontends are independently selectable. The cmocka test dependency is tied to `BUILD_SUITE`. Utility dependencies enabled in the library are real library dependencies, even when the UI is absent. [Feature configuration](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L54-L72), [test dependency](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L499-L521), [vendored source](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L109-L110), [core composition](https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/CMakeLists.txt#L917-L988). |
| **jgenesis Game Boy core** | This Rust core uses `std`. Its manifest depends on `bincode` with derive support, `log`, `rand`, `thiserror`, and internal common/config/DSP/macro crates. Its API receives rendering, audio, input, and save services through frontend traits. | GPU rendering, SDL3 host integration, GUI, and WebAssembly frontend live in separate crates. The project demonstrates that a Rust `std` core with selected generic dependencies can retain distinct native and browser frontends. [Exact core manifest](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/Cargo.toml), [use of std](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/src/lib.rs#L17-L29), [embedding API](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/src/api.rs#L211-L234), [architecture](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/ARCHITECTURE.md#L126-L150). |

The shared lesson is to keep host integration out of the hardware engine and
make dependency costs visible. Their dependency counts differ. The C examples
establish those architectural boundaries; C use of libc does not directly settle
Rust's `std` versus `no_std` choice.

## Three independent choices

- **`no_std`** changes Rust's implicit standard-library linkage and prelude.
  It does not prohibit third-party crates, and dependencies can still link
  `std`. [Rust Reference](https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-attribute)
- **No third-party runtime dependencies** concerns the dependency graph. It does
  not imply no standard library, no allocation, or no generated code.
- **No allocation during ordinary execution** concerns which operations the
  running core performs. A `std` core can satisfy it; a `no_std` core can allocate
  through `alloc`, which provides `Box`, `Vec`, and other heap-backed facilities.
  [Rust alloc documentation](https://doc.rust-lang.org/alloc/)

Browser support does not require `no_std`. Rust's `wasm32-unknown-unknown` target
supports `std`, `core`, and `alloc`; OS-dependent functions such as filesystem
access and thread creation are unavailable or fail. Keeping those services in
the frontend is the relevant boundary.
([Rust target documentation](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html))

Build tools and procedural macros run during compilation; development
dependencies support tests/examples/benchmarks rather than ordinary downstream
use. They still have build-time and maintenance costs, while generated code can
have runtime costs. Count those separately instead of presenting one dependency
total as a performance result.
([Cargo dependency kinds](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#development-dependencies),
[procedural macros](https://doc.rust-lang.org/reference/procedural-macros.html))

## HachiStep recommendation

Keep **`std` available, a small justified dependency set, and no allocation during
ordinary emulation**. These choices are compatible. The starter already uses the
default `std` environment and currently declares no third-party dependencies;
zero is its present state, not a design requirement.
([Core manifest](../../crates/hs-core/Cargo.toml),
[crate entry point](../../crates/hs-core/src/lib.rs),
[allocation contract](../DESIGN.md#193-allocation-and-memory))

1. Add a dependency when it replaces substantial generic work and its actual
   enabled code is appropriate for the native and browser targets. Compare
   maintainability, generated code, transitive features, footprint, and measured
   runtime cost. Do not maintain a homegrown generic subsystem merely to preserve
   a zero-dependency count.
2. Keep files, wall-clock access, host entropy, threads, windowing, audio devices,
   and platform bindings outside hardware execution. Inputs remain explicit;
   deterministic behavior follows from that boundary, not from `no_std`.
3. Permit allocation at construction and explicit save-state operations while
   preserving the existing ordinary-run contract. A serializer's occasional
   buffer allocation is different from allocating per instruction or edge.
4. Do not add a `no_std` target promise or another configuration matrix now.
   Revisit it when a real consumer requires it. Avoid needless platform coupling
   so that decision remains practical.
5. Keep the existing `forbid(unsafe_code)` policy for this crate. It does not
   claim that `std` or third-party implementations contain no internal unsafe
   code. Choosing a dependency through a safe API is a separate decision.
6. The save-state candidate below makes the proposed core dependency concrete.
   This research does not install packages; the emulator examples above are
   evidence for reuse, not a commitment to their particular dependencies.

## Candidate evaluation: native save-state encoding

**Keep `borsh` 1.8.1 as the leading candidate for `hs-core`, with `std` and
`derive` only.** Final selection follows the hardware-oriented saved-state
contract, not convenience when deriving on current structs. This candidate has
not been added to Cargo manifests. Current releases and
upstream maintenance were checked on 2026-09-16; Borsh 1.8.1 was published on
2026-08-26. ([Published release](https://docs.rs/crate/borsh/1.8.1),
[upstream](https://github.com/near/borsh-rs))

```toml
borsh = { version = "1.8.1", default-features = false, features = ["std", "derive"] }
```

Borsh gives fixed-width integer encoding with little-endian byte order, including
the core's `u128` clock values. Struct fields follow declaration order; enum tags
and contents are encoded explicitly. These are language-independent byte rules;
Borsh is not self-describing, so readers must separately agree on the structure
and meaning of the data. Those meanings must describe the saved hardware and
restoration guarantee. A private continuation enum remains private semantics
even when another language can decode its tag. Use fixed-width Rust integers
for persisted values; the guest CPU's byte order need not determine the save
format. ([Format specification](https://github.com/near/borsh#specification))

Its const-generic array and `Box` implementations directly cover the starter's
49,152-byte flash, 65,536-byte EEPROM, and nested `[[i16; 3]; 64]` sensor history.
Byte arrays use bulk writes/reads. This removes the need for separate array
helpers, but does not establish that all current internal fields belong in the
saved contract. Codec convenience is only one selection criterion.
([Encoding implementation](https://github.com/near/borsh-rs/blob/fe778bec428d5b44cd4922c896af0a7c39a863dd/borsh/src/ser/mod.rs#L579-L603),
[decoding implementation](https://github.com/near/borsh-rs/blob/fe778bec428d5b44cd4922c896af0a7c39a863dd/borsh/src/de/mod.rs#L766-L850))

Define completed effects, retained values, operation progress, and remaining
clock obligations first. Derive only on types whose fields and tags express that
contract. Otherwise map compact runtime state to and from that description during
capture/restore, without forcing an extra representation into ordinary execution.
Generation can share these mappings; a second machine-wide state hierarchy is
not automatically necessary. Where appropriate, `#[borsh(skip)]` restores omitted
derived fields through `Default`; omission must follow the semantic contract.

Decode an owned candidate with `from_slice` (which rejects trailing bytes), check
hardware/state invariants, rebuild derived fields, and only then replace the live
machine. Codec validation does not establish hardware validity or interoperability.
Capture and encoding remain explicit operations at the stopped API boundary,
with no guest execution or ordinary-run bookkeeping. Add no state versions or
compatibility layer. Evaluate exact resumption, state coverage, bounded loading,
and practical capture/load costs before committing the codec.
([Skip behavior](https://docs.rs/borsh/1.8.1/borsh/derive.BorshDeserialize.html),
[complete-input decoding](https://github.com/near/borsh-rs/blob/fe778bec428d5b44cd4922c896af0a7c39a863dd/borsh/src/de/mod.rs#L1034-L1045),
[state contract](../DESIGN.md#136-native-save-state-design))

| Alternative | Assessment for this core |
| --- | --- |
| **Postcard 1.1.3 + Serde** | Maintained and viable. Its larger integers use specified variable-length encoding, which is deterministic but differs from fixed-width wire fields. Serde's large fixed arrays need helpers/adapters; the starter also has boxed arrays and nested non-byte arrays. This adds integration work without an existing core need for Serde's broader format ecosystem. [Wire specification](https://postcard.jamesmunns.com/wire-format.html), [array support](https://docs.rs/serde-big-array/0.5.1/serde_big_array/), [upstream](https://github.com/jamesmunns/postcard). |
| **Bincode** | Reject for a new dependency: upstream explicitly ceased maintenance. The latest 3.0.0 package is a notice and compiler error; 2.0.1's native derives and configurable encoding do not remove that concern. [Maintainer notice](https://docs.rs/crate/bincode/3.0.0). |
| **Bitcode 0.6.9** | Maintained, viable native derives with arrays and field skipping. It groups fields and packs integer values, and brings `bytemuck` plus derive tooling. Those mechanisms may help state throughput/size, but that benefit has not been established for this workload. Prefer Borsh's straightforward encoding here; compatibility promises are not the deciding factor. [Implementation overview](https://docs.rs/bitcode/0.6.9/bitcode/), [manifest](https://github.com/SoftbearStudios/bitcode/blob/f41da053c08178189aaee8c62f4c6e738add6eda/Cargo.toml). |

The proposed Borsh features do not enable schema generation or optional collection
integrations. They do add build dependencies: `cfg_aliases`, and procedural macro
tooling through `borsh-derive` (`syn`, `quote`, `proc-macro2`, `proc-macro-crate`,
`once_cell`, and their transitives). This is one direct package, not a one-package
graph. Borsh declares Rust 1.77; implementation must replace the starter's 1.74
minimum and check the resolved macro graph's requirements. No locked-graph build
was performed. ([Runtime/build manifest](https://github.com/near/borsh-rs/blob/fe778bec428d5b44cd4922c896af0a7c39a863dd/borsh/Cargo.toml),
[derive manifest](https://github.com/near/borsh-rs/blob/fe778bec428d5b44cd4922c896af0a7c39a863dd/borsh-derive/Cargo.toml))

One existing representation needs deliberate handling: `Machine.fault` blocks
future execution, but its `Error` contains static string references. Define native
capture/restore handling of this terminal status without accidentally standardizing
emulator diagnostics as hardware state. Silently skipping it would change native
session behavior after restore. This is independent of codec choice. At
implementation, check large-array decode stack/copy behavior on native and browser
targets and save/load continuation around partial operations. No measured speed
advantage or cross-emulator restoration guarantee has been established.
([Machine state](../../crates/hs-core/src/machine.rs),
[error representation](../../crates/hs-core/src/error.rs))

## Concrete tooling and verification recommendations

The starter already maintains generic code that libraries can replace. These
are proposed direct dependencies with specific integration work, not placeholder
packages to add to otherwise unchanged manifests.

| Package and location | Work it replaces or supplies | Planned integration |
| --- | --- | --- |
| **`sha2` 0.11, CLI** | The complete handwritten SHA-256 implementation in [digest.rs](../../crates/hs-cli/src/digest.rs), including padding and compression rounds. | Replace it with the library's `Sha256` API, retaining the existing output identities. Use its portable software backend. [API and backend selection](https://docs.rs/sha2/0.11.0/sha2/). |
| **`serde_json` 1, CLI** | Manual string escaping, array formatting, and the large JSON format string in [output.rs](../../crates/hs-cli/src/output.rs). | Express the report with `json!` over its primitive values and serialize it. This use does not need a direct Serde dependency or serialization derives throughout the hardware core. [JSON construction](https://docs.rs/serde_json/1.0.151/serde_json/macro.json.html). |
| **`clap` 4 with `derive`, CLI** | Separate option declarations, defaults, parsing, and help text in [main.rs](../../crates/hs-cli/src/main.rs). | Define the two commands and their options together. Enable `std`, `derive`, `help`, `usage`, and `error-context`; leave other features off initially. [Derive API](https://docs.rs/clap/latest/clap/), [features](https://docs.rs/clap/latest/clap/_features/index.html). |
| **`proptest` 1, development only** | Handwritten pseudo-random call splitting in [kernel.rs](../../crates/hs-core/tests/kernel.rs) and [retail.rs](../../crates/hs-core/tests/retail.rs), which provides no automatic reduction of a failing split sequence. | Generate valid run partitions and save/restore positions, then shrink failures to small reproducers. Compare subsequent observable behavior. Use `std` without the default fork, timeout, and bit-set features. [Generation and shrinking](https://proptest-rs.github.io/proptest/intro.html), [feature definitions](https://github.com/proptest-rs/proptest/blob/main/proptest/Cargo.toml). |

The three CLI replacements are already justified and can be one implementation
change. Define the saved-state contract and finalize its codec when implementing
native save-state encoding; Borsh is the leading candidate. Add Proptest with the
partition and resume properties. Those properties test execution and persistence contracts;
the independent hardware suite continues to supply hardware expectations.

The build-time CPU generator can start with typed Rust data and standard-library
text emission. It does not yet need a parser framework or direct `syn`/`quote`
dependencies. Existing error formatting is small enough to keep explicit for now;
`thiserror` was considered, but its boilerplate savings are less substantial than
the selected replacements. ([Thiserror's generated implementations](https://docs.rs/thiserror/latest/thiserror/))

This plan identifies four tooling/development packages and shortlists Borsh for
the core, each with transitive dependencies. It introduces no new runtime services into the
executor. Update the compiler minimum to match the selected locked graph: these
packages exceed the starter's 1.74 declaration. The local compiler checked
during this interview is 1.98.1. No package integration or build measurement has
been performed during the design interview.
