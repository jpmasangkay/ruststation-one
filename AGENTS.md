# Engineering Guardrails & Architecture Guide (`AGENTS.md`)

Primary specification: [`docs/ps1_emulator_blueprint.md`](docs/ps1_emulator_blueprint.md).

All contributors and AI agents working in this repository MUST enforce the following invariants without exception:

## 1. Hard Architectural & Safety Invariants

- **Strict `no_std` Core Isolation:** `crates/psx-core` must always compile with `#![cfg_attr(not(feature = "std"), no_std)]` and `extern crate alloc;`. Never add OS, filesystem, wall-clock, thread, audio driver, or windowing dependencies to `psx-core`. Verify with `cargo check -p psx-core --no-default-features`.
- **Zero `unsafe` in Phase 1:** `crates/psx-core/src/lib.rs` enforces `#![forbid(unsafe_code)]`. Any future JIT or SIMD `unsafe` blocks (Phase 4+) must live in dedicated, feature-gated crates/modules with explicit `// SAFETY:` proofs.
- **Single Master Clock & Bounded Scheduler:** All hardware subsystems step from the single monotonic 33.8688 MHz `EventScheduler` (`MASTER_CLOCK_HZ = 33_868_800`, `MAX_EVENTS = 32`, `MAX_CPU_SLICE_CYCLES = 128`). Never use unbounded queues, unbounded loops, or unbounded recursion.
- **Zero-Allocation, Zero-`dyn` Hot Paths:** Forbid heap allocations (`Vec::push`, `Box::new`, `format!`) and dynamic dispatch (`dyn Trait`) inside `Cpu::step_instruction`, `Bus` read/write paths, `Gte::execute`, and `System::step_frame`. Use monomorphized generics (`System<G: GpuBackend>`).
- **Assert Hardware Invariants:** Maintain side-effect-free `debug_assert!` / `assert!` preconditions and postconditions on state transitions (e.g., `regs[0] == 0` before and after every instruction, aligned bus word accesses, monotonic scheduler timestamps).

## 2. Clean-Room & Legal Hygiene

- **Workspace License:** `MPL-2.0` (see [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)).
- **Zero Proprietary Assets:** Never commit Sony BIOS ROMs (`scph*.bin`) or commercial game binaries/ISOs. Local test files belong exclusively in gitignored `local/bios/` and `local/test_roms/`.
- **Clean-Room Reference Rule:** Never copy or adapt code from post-September-2024 DuckStation (`CC BY-NC-ND 4.0`). Base hardware implementations on [PSX-SPX](https://psx-spx.consoledev.net/), PCSX-Redux `OpenBIOS` (MIT), and bare-metal hardware test suites.

## 3. Mandatory Verification Gate Before Committing

Every change must pass all four checks cleanly:
1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo check -p psx-core --no-default-features`
