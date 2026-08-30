# RFC 0017: A property that cannot have a fixture gets a build instead

- Status: accepted
- Date: 2026-08-30
- Affects: `kernel/`, `xtask/`, `docs/design/ring-scene-boot.html` section 15 M4

## Decision

The kernel gains a Cargo feature that makes it wrong on purpose, and
`cargo xtask mutate` is the harness that requires the wrongness to be caught.
There is one such feature — `mutate-unchecked-index`, which removes the bounds
check from the capability table's handle lookup — and the harness boots a kernel
built with it into the forging sweep, requires the boot to go red *with a kernel
panic in the log*, then boots the same command line without the feature and
requires it to go green. `cargo xtask verify` runs it, and
`cargo xtask lint-mutations` refuses a manifest that turns any such feature on
by default.

## Context

E0-P08's exit criterion has two halves: the five properties of the capability
negative suite hold, **and** each of them has a mutation that makes it fail. The
second half is what says the suite can fail at all, and a suite whose checks
cannot fail is a suite nobody has tested.

Four of the five are met by fixtures. `kernel::cap::properties::Flawed` builds
five tables broken on purpose, one per property, and `properties::check` is
required to catch each with the property it breaks and no other. That runs at
every boot.

The fifth property is *a process cannot make the kernel panic by trying*, and a
fixture cannot express it. A table built to panic takes the machine down when it
is exercised, rather than being caught by the harness exercising it — and there
is no host test harness for kernel logic to catch it in, because the kernel is
`no_std` and defines its own panic handler, which `kernel/Cargo.toml` sets out.

So the fifth had a compile-time half and half of a runtime one: the module
denies `clippy::indexing_slicing`, `unwrap_used`, `expect_used`, `panic` and
`unreachable`, so the constructs that turn a hostile handle into a fault cannot
be written by accident; and one flawed table covers the *masked*-index form of
the mistake, which is refusable rather than fatal. What was missing was the form
that actually panics. E0-P08 was marked `[>]` rather than met, with that gap
named.

Three alternatives were live.

**Move the capability table out of the kernel crate so a host test can panic
safely.** This is the reversal `intent/0003`'s risk section already named, and it
is the right answer eventually. It is also a bigger decision than this one: it
splits the frame across crates, and the frame is the thing the `< 5% unsafe of
TCB` metric is measured over. Doing it to gain one test is doing it for the
wrong reason.

**Catch the panic in the kernel.** There is nothing to catch it with. `panic =
"abort"`, no unwinding, and a panic handler whose only correct behaviour is to
report and stop — turning it into something recoverable would make a panic a
mechanism rather than a bug, which `main::panic` refuses in as many words.

**Leave it at the compile-time half.** Defensible, and it is what M4 shipped. It
is also weaker than it looks: `deny` is not `forbid`, an `#[allow]` on one
function silences it, and nothing would notice. The mutation build is what
notices.

## Consequences

Easy: property five is falsifiable. The harness demonstrates, on every `verify`,
that a kernel which subscripts a handle's index instead of checking it fails the
suite — and that the same suite passes when it does not. That is the second half
of E0-P08's exit criterion, met by the mechanism the criterion itself named.

Also easy: the pattern extends. `MUTATIONS` in `xtask` is a list, and a property
that later turns out to be unfixturable gets an entry rather than an argument.

Hard, and stated rather than hidden: **the defect is in the shipped source.** It
is behind a feature that is off by default, in one function, marked with an
`allow` that names itself, checked by a lint that refuses to let it become a
default — but it is there, and a reader of `kernel/src/cap.rs` sees two versions
of one lookup. That is the same trade `properties::Flawed` already makes and it
is worth being explicit that this makes the trade twice.

The cost of `verify` goes up by two kernel builds and two boots, because a
feature change invalidates the build. The mutated boot runs first so that the
tree is left holding a clean one.

## What would reverse this

A host harness for kernel logic. The moment the capability table can be tested
on the host — which means it moving out of the kernel crate, which means the
frame being split — property five gets a fixture like the other four, this
feature is deleted, and `xtask mutate` loses its only entry. That is E1's
supervisor work and the trigger for it is a second thing that needs the same
treatment, not this one.

The other reversal is the harness proving unfalsifiable in practice: if a
mutation build starts failing for a reason other than the defect — a link error,
a boot that never reaches the sweep — the harness is asserting on the wrong
thing, and the answer is a narrower assertion rather than a louder one. That is
why it checks the log for a panic rather than accepting any red boot.
