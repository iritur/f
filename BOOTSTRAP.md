# Bootstrap

## Honest status

**It compiles, it boots, and it says so.** `cargo xtask lint`, `test`,
`coverage` and `run` are all green, and `cargo deny check` passes. The kernel is
loaded by a multiboot loader, walks to long mode, reports the memory map it was
handed, checks the determinism substrate against itself and exits 33.

Two runs of the same commit produce byte-identical serial output, which is the
weakest useful form of the M0 contract and the one everything else rests on.

This page said "never been compiled" until the first green build, and the
prediction it made then turned out to be right: what needed fixing was syntax
and toolchain friction rather than structure. The count was thirteen small
things — two policy lints that matched their own source, three compile errors, a
clippy rule the kernel broke, a dependency-policy config key that had been
removed upstream, a litmus test that raced nothing and reported corruption on a
healthy ring, a test suite that named three crates by hand and so ran neither
`f-bench` nor `f-init`, and the percentile convention in the measurement
harness. None of them were architectural. That is the number worth knowing
before writing the next scaffold this way.

## Prerequisites

```
# toolchain — the pin in rust-toolchain.toml is fetched automatically
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# emulator
#   Debian/Ubuntu:  sudo apt install qemu-system-x86
#   macOS:          brew install qemu
#   Windows:        winget install SoftwareFreedomConservancy.QEMU

cargo install cargo-deny     # dependency policy, matches the CI job
```

## First run

```
cargo xtask lint     # the four policy checks — expect these to pass first
cargo xtask test     # abi, env, ring, init on the host
cargo xtask run      # kernel in QEMU; expect exit 33 and "M0 ok"
```

## Known gaps at M0

Each of these is deliberate and has a home in the milestone ladder
(`docs/design/ring-scene-boot.html` section 15).

| Gap | Where it lands |
|---|---|
| No framebuffer. The multiboot 1 handoff delivers a memory map and nothing else, which is what M1 needs and all it needs. A framebuffer arrives with the compositor's protocol, not before. | phase 03 |
| The boot protocol is BIOS-era and QEMU-shaped. Multiboot 1 was chosen because QEMU implements it in its own loader — no vendored bootloader binary, no ISO step, one command. The machine named at E5 will want Limine or UEFI, and the handoff is one pointer wide so that swap is two files. | E5 |
| Every page is 2 MiB and every mapping is writable. No read-only text, no 4 KiB granularity, no per-process address spaces. Read-only text needs either 4 KiB granularity or 2 MiB-aligned sections in the linker script, which is the same prerequisite as the guard page below — the two arrive together. Data is no longer executable: the direct map is mapped no-execute, and `cargo xtask fault nx` proves it by faulting. | M3-M4 |
| `targets/x86_64-f.json` is shipped but unused — `xtask` builds against the built-in `x86_64-unknown-none`. Switch when the target needs something the built-in does not give. | M1 |
| No descriptor tables, no paging, no APIC. | M1-M2 |
| `Env` has a seeded implementation only. The hardware implementation reads the one legitimate `rdtsc` but is not yet wired to it. | M2 |
| No capability table. The negative test suite is the phase-00 exit criterion and does not exist yet. | M4 |
| `Producer`/`Consumer` are bound to borrowed memory, not to a mapped shared region with a validated `ChannelHeader`. | M5 |
| Coverage instrumentation for fuzzing. Cheap now, painful later — see `docs/design/proving-ground.html` section 08. | M0-M1 |
| AArch64 target builds, but nothing is verified on it. The CI job exists so that stops being true early. | M5 |

## What is already real

- The determinism substrate, its reproducibility tests, and the lint that keeps
  it honest. This is the piece that cannot be retrofitted, so it exists first.
- The ring cursor protocol with the correct acquire/release pair, cache-line
  separated cursors, and tests for wrap-around, a full ring, a hostile cursor
  and doorbell suppression.
- The ABI layout assertions. A change that breaks `size_of::<Sqe>() == 64`
  fails at compile time in `abi/` rather than at a peer.
- Four decisions as executable checks rather than prose: determinism, the
  licence boundary, the frame's `unsafe` allow-list, and kernel state sharded
  per core behind `PerCpu<T>` while only one core is running.
