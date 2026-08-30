---
id: 0004
status: done
spec: ./spec.md
---

# Plan: the second core, the shootdown, the component, and the mutation build

One change, one pull request, for the reason intents 0002 and 0003 gave and
which is sharper here: none of the four things this delivers can be observed
without the first. The shootdown needs a core to shoot down to. The component
needs a core to run on. The mutation build needs a suite that still passes. Four
pull requests would be one that works and three that cannot say whether they do.

## Files

```
kernel/linker.ld                     a stack block per core: guard, stack,
                                     guard, fault stack, and the symbols the
                                     kernel derives the geometry from
kernel/src/arch/x86_64/ap.rs         NEW: the trampoline, the startup sequence,
                                     the interrupt command register, and where
                                     a started core's stacks are
kernel/src/smp.rs                    NEW: the mailbox, the handoff, where a
                                     core arrives, the parking loop, and the
                                     shootdown protocol
kernel/src/arch/x86_64/paging.rs     `unmap_user_live`; the on-ramp mapping and
                                     its withdrawal, which gives back the
                                     tables it took; `BuildError::NotMapped`;
                                     the started cores' guard pages skipped
kernel/src/arch/x86_64/apic.rs       `SHOOTDOWN_VECTOR`; `window` and
                                     `tsc_khz`, so another module can address
                                     this core's APIC; `end_of_interrupt`;
                                     `adopt`, which is `init` less everything
                                     that is not per core
kernel/src/arch/x86_64/idt.rs        the shootdown stub, its gate, and the
                                     branch in the dispatcher
kernel/src/arch/x86_64/gdt.rs        a double-fault stack per core, and the
                                     comment that predicted it corrected
kernel/src/arch/x86_64/mod.rs        `cpuid_subleaf`, for the topology leaf;
                                     the `current_cpu` reversal that did not
                                     happen, corrected rather than left
kernel/src/arch/x86_64/multiboot.rs  `Module::bytes`, and what makes the
                                     lifetime honest
kernel/src/cap.rs                    `Slot::mapped`; `Revoked`, which says what
                                     a revocation withdrew as well as how much;
                                     `note_mapping`; `Table::of`; and the one
                                     place the table is subscripted, in two
                                     versions
kernel/src/process.rs                `run` split into `prepare`, `execute` and
                                     `reap` across two cores; `Plan`; `Job`;
                                     `Outcome`; `withdraw`; the eighth
                                     provocation; the tally that depends on
                                     what ran before
kernel/src/arch/x86_64/probe.rs      handles read from the entry word rather
                                     than assumed, and the `cap=unmap` block
kernel/src/main.rs                   bring-up, two processes inside one window,
                                     and the module the loader placed
kernel/Cargo.toml                    the deliberate defect, off by default
abi/src/door.rs                      NEW: the call numbers, the calling stub,
                                     the argument packings, and `Entry` — what
                                     a component is told rather than assumes
abi/src/lib.rs                       the module
user/init/src/component.rs           NEW: the component, in safe Rust
user/init/src/lib.rs                 the module, and what the crate now is
user/init/link.ld                    NEW: the image, and why the placement is
                                     here rather than an attribute
user/init/Cargo.toml                 why it is a library and not a binary
Cargo.toml                           `[profile.init]`, and why it exists
xtask/src/main.rs                    `init` and `mutate`; `lint-mutations`;
                                     `-smp 2` and `-initrd`; the eighth escape;
                                     one machine definition with a capturing
                                     variant
docs/rfc/0016-what-crosses-a-core.md NEW: four words, and nothing else
docs/rfc/0017-a-kernel-that-can-be-built-wrong.md
                                     NEW: a property with no possible fixture
intent/0004-…/                       NEW: this intent, its spec and this plan
TODO.md                              E0-B10 done, E0-P08 met, and the four
                                     places that said revocation leaves the
                                     mapping
CLAUDE.md                            two commands, and the per-CPU convention
                                     as amended
docs/TESTING-STATUS.md               what has now been executed
.github/workflows/ci.yml             the mutation harness joins the gate
```

## Order

Whatever cannot be observed until something else exists, last. Each step was
taken to a green `cargo xtask verify` before the next one started.

1. **The second core.** Linker script, `ap.rs`, `smp.rs`, the on-ramp, `-smp 2`.
   Nothing uses it yet; the boot log says how many cores started and the timer
   assertion says core 0 still kept its schedule. This is the step that could
   have failed in a way that changed the shape of everything after it, which is
   why it is first and alone.
2. **The process moves.** `run` split three ways, the job posted through the
   mailbox, the running core arming its own timer. All seven `user=` and seven
   `cap=` boots pass from a core that is not the one holding the window.
3. **The shootdown, and revocation reaching the mapping.** `unmap_user_live`,
   the vector, the protocol, the slot's recorded address, and `cap=unmap` — the
   eighth escape, and the only one answered by the processor rather than by the
   table.
4. **The component.** `abi::door`, `user/init` as an image, the linker script,
   the profile, `cargo xtask init`, and the kernel running two processes.
5. **The mutation build.** The defect, the harness, the lint, and `verify`
   gaining a fourth step.
6. The documents.

## What this expected to be wrong about, and what it was wrong about

Named in advance, so that being wrong is a finding rather than a surprise. Three
were predicted; two were not, and both of the unpredicted ones are the
interesting ones.

**Predicted, and it happened.** *The trampoline's addressing.* Code assembled at
one address and executed at another cannot use a link-time reference to itself.
Every address in it is a literal or a difference of two labels in the same
section, and the one that would have been silent — a sixteen-bit `lgdt`
truncating the descriptor table's base to twenty-four bits — is why it is
`lgdtl`.

**Predicted, and it did not happen.** *The `GS`-based core index.* A comment in
`arch/x86_64/mod.rs` predicted that E0-B10 would move `current_cpu` out of
`cpuid` and into `GS`. It did not, and the comment now says why rather than
being quietly left: `GS` is already the ring-3 entry block, the swap between the
two halves happens on the system-call path and *only* there, and the interrupt
stubs do not swap. A core index in `GS` would be right in a system call and
would read a process's base in the timer handler, which is the one caller on the
critical path. Making it right means `swapgs` in every stub, which is a change
to the interrupt entry path rather than to that function.

**Not predicted: a component cannot name its own entry point.** `user/init`
inherits `unsafe_code = "forbid"`, and in this edition `#[unsafe(no_mangle)]` and
`#[unsafe(link_section)]` are unsafe *attributes* — the lint does not
distinguish an attribute whose hazard is a duplicate symbol from a dereference
of a wild pointer, and forbid cannot be overridden by an `allow`. So the crate
cannot be a binary, and cannot mark a function as the entry.

The answer is that the placement belongs to the linker script anyway: the
component is a library, `link.ld` puts the section its entry was compiled into
at the image's first byte, and `cargo xtask init` checks that the symbol which
actually landed there is that one — so a toolchain that changes how it names
sections breaks the build with a sentence instead of producing an image that
starts in the middle of something.

Two further things fell out of that and both cost a cycle. Link-time
optimisation leaves a library's rlib carrying bitcode rather than machine code,
because the final artifact is expected to do the optimising — and there is no
final artifact here, so the image linked to *nothing*, silently, and the failure
looked exactly like the entry point having moved. Hence `[profile.init]`. And a
`staticlib` crate type, which is the obvious way to hand the linker one file, is
built for the host too, where a `no_std` crate has no panic handler to borrow and
the test profile insists on unwinding.

**Not predicted: a second process cannot know its own handles.** The probe had
its three starting handles as constants, at the first generation, because the
frame grants into a cleared table in a fixed order and a process was said to be
*entitled* to know them. That is true of the first process on a core and false
of the second: generations survive `clear_all` — which is the whole point of
them, and the one boundary where resetting would be most tempting — so the
second process finds its capabilities at the same indices and a later
generation.

Found the way it should be found: the component ran correctly, the adversary
that followed it was refused on its very first call, and the boot said so. The
answer is `door::Entry`, one register in which the frame *tells* a component the
first handle it was granted. It is the smallest possible version of something a
component will eventually be sent on a channel, and it retires a sentence in
three files.

It also made the forging sweep's expectation depend on what ran before, which is
better rather than worse: a handle at a generation below the slot's is refused as
*revoked* rather than as unknown, so the tally now distinguishes "you had this
once" from "this never existed" — and every boot checks that a handle the
previous process on that core held does not resolve.

**Also found, and worth recording as a scar.** The `#[allow]` marking the
deliberate defect is load-bearing in an unexpected direction: `deny` and not
`forbid` on the module is what makes the mutation build possible at all. If that
module's lint level is ever tightened to `forbid`, the mutation harness stops
being buildable and property five loses its second half — quietly, because the
harness would fail to compile rather than fail to catch.
