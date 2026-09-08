// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The frame's whole share of `f.root=`: read the token, hand the modules to
//! `f-generation`, and print what came back.
//!
//! # Why the frame reads a token at all
//!
//! `abi/src/boot.rs` was written with nothing parsing it and one sentence
//! naming the day: *the frame does at `E2-P07`*. This is that day. Selection
//! has to happen below everything, because at boot the store is not running and
//! an assembler holding a root hash has nothing to read — so the generation
//! travels as a multiboot module and the only thing that can say *this is the
//! one you asked for* is the fold over the records inside it.
//!
//! # Why this file has no decision in it
//!
//! Every branch that could be wrong is in `f_generation::select`, which is a
//! `no_std` library with a host test suite. `kernel/` is `test = false` — the
//! harness links `std` and a second crate would claim `panic_impl` — so a rule
//! written here could only ever be exercised by booting QEMU, and a refusal
//! that only a boot can reach is a refusal nothing checks. What is left here is
//! a loop over the loader's modules and a block of `kprintln!`.
//!
//! # What this does not do, and the line is RFC 0066's
//!
//! It does not instantiate a topology. `f-assembler` is the reader of a boot
//! module's *contents* and it is not a component yet: nothing at boot calls it,
//! there is no supervisor to be its caller, and an image linking it needs a
//! `#[global_allocator]`, which is an `unsafe impl` and forbidden above the
//! frame. So the frame *selects* and reports, and instantiation stays where
//! RFC 0066 left it. A frame that started checking each file against its leaf
//! would be the second implementation of the assembler's job, which is the one
//! thing the whole boot-module arrangement exists not to have.
//!
//! # Why nothing is printed on an ordinary boot
//!
//! The boot log is a fixture: `cargo xtask trace` hashes it and two runs of one
//! commit must produce the same bytes. A machine that was not asked for a
//! generation prints nothing at all here, so every boot that existed before
//! this file still has the log it had. `boot_time`'s doc comment makes the same
//! argument about a duration, and it is worth making twice.

use f_abi::boot::Selection;
use f_generation::select::{Chosen, select};

use crate::arch::x86_64::multiboot::BootInfo;
use crate::kprintln;

/// The most modules a machine can offer, which is the loader's own bound.
///
/// Restated from `multiboot::MAX_MODULES` rather than imported, because a
/// second array sized by that constant is a place a change to it could go
/// unnoticed; the assertion below is what makes this a copy that cannot drift.
/// Unit: count of modules.
const OFFERED_MAX: usize = 8;

/// What the frame decided about the generation it was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selected {
    /// No `f.root=` on the command line. An ordinary boot, and not a boot that
    /// asked for something and did not get it.
    Unasked,
    /// The token was there and the machine is the generation it named.
    Ran(Chosen),
    /// The token was there and this build could not read it. Carries the packed
    /// `f_abi::error` the grammar refused with.
    Malformed(i32),
    /// The token was there, was read, and no module the loader offered folds to
    /// it. Carries what was on offer, so the log can say *five modules, two of
    /// them generations, neither of them this one*.
    Missing(Chosen),
}

/// Read the token, choose a module, and say what happened.
///
/// # Safety
///
/// The direct map must be live and `frames` must already have been rebound onto
/// it. That is [`crate::arch::x86_64::multiboot::Module::bytes`]'s obligation,
/// discharged here the same way `component::modules` discharges it: every
/// module is in the reserved list — see `main::reserved_ranges` — so nothing
/// else owns these bytes, and the caller has passed the point where the
/// allocator was populated from that list.
#[must_use]
pub unsafe fn selected(boot: &BootInfo) -> Selected {
    let Some(parsed) = Selection::find(boot.cmdline()) else {
        return Selected::Unasked;
    };
    let asked = match parsed {
        Ok(selection) => selection.root,
        Err(why) => return Selected::Malformed(why),
    };

    // A fixed array rather than an iterator over the loader's list, because
    // `Module::bytes` is `unsafe` and one `unsafe` block over a bounded loop is
    // cheaper to discharge than one inside a closure a library calls back into.
    let mut offered: [&'static [u8]; OFFERED_MAX] = [&[]; OFFERED_MAX];
    let mut count = 0;
    for module in boot.modules() {
        if count == OFFERED_MAX {
            break;
        }
        // SAFETY: the caller's guarantee. Every module the loader reported and
        // this build kept is in the reserved list, so the direct map covers this
        // extent and nothing else is writing it.
        offered[count] = unsafe { module.bytes() };
        count += 1;
    }

    let chosen = select(&asked, offered[..count].iter().copied());
    if chosen.at.is_some() { Selected::Ran(chosen) } else { Selected::Missing(chosen) }
}

/// Print it, and say whether the boot may continue.
///
/// Returns `false` for a machine that was asked to be a generation it cannot
/// be. That is a failure and not a warning: `f.root=` is how a rollback names
/// the generation to go back to, and a machine that quietly booted a different
/// one would be the exact failure `E2-P07` is a test for — the operator
/// believes they rolled back and the evidence says nothing.
pub fn report(selected: Selected) -> bool {
    match selected {
        Selected::Unasked => true,
        Selected::Ran(chosen) => {
            let (Some(found), Some(at)) = (chosen.found, chosen.at) else {
                // Unreachable by construction — `selected` builds `Ran` only
                // where both are set, and they are set together — and reported
                // rather than unwrapped, because a panic in the frame stops the
                // machine and this is a print.
                kprintln!("FAIL: the generation: a selection with nothing selected");
                return false;
            };
            kprintln!("  generation    selected — one offered module folds to the root asked for");
            kprintln!("    root        {}", Hex(&found.root));
            kprintln!("    bytes       {}", Hex(&found.digest));
            kprintln!(
                "    module      loader's module {at}, {} byte(s), {} member(s)",
                found.bytes,
                found.members
            );
            kprintln!(
                "    offered     {} generation(s), {} refused",
                chosen.offered,
                chosen.rejected
            );
            true
        }
        Selected::Malformed(why) => {
            kprintln!(
                "FAIL: the generation: `f.root=` is on the command line and is not \
                 sixty-four lower-case hexadecimal characters ({why:#x})"
            );
            false
        }
        Selected::Missing(chosen) => {
            kprintln!(
                "FAIL: the generation: no module this machine was offered folds to the root \
                 it was asked for"
            );
            kprintln!(
                "    offered     {} boot module(s), {} of them refused",
                chosen.offered,
                chosen.rejected
            );
            if let Some(why) = chosen.why {
                kprintln!("    first       {}", why.message());
            }
            false
        }
    }
}

/// Sixty-four lower-case hexadecimal characters, without an allocator.
///
/// The frame has no formatter for a byte slice and `f_abi::boot::Selection`
/// renders a whole token rather than a bare hash, so this is the smallest thing
/// that prints one. Lower case only, for the reason that module gives: a hash
/// printed one way and parsed another is a hash two people compare by eye and
/// disagree about.
struct Hex<'a>(&'a [u8; 32]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            write!(out, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The loader's bound and this file's copy of it are one number.
const _: () = assert!(OFFERED_MAX == crate::arch::x86_64::multiboot::MAX_MODULES);
