// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What this supervisor was given and what it is allowed to fill, written by
//! the frame and read by the component, in one page neither of them has to
//! guess the shape of.
//!
//! # Why a supervisor is told rather than compiled
//!
//! `user/virtio-blk/src/routing.rs` makes this argument for a driver and every
//! word of it holds here, but the sharpest case is a different one. A driver is
//! told where its device landed because the *device* chose. A supervisor is
//! told which account it may spend and which manifest it may put into which
//! place because **the frame chose, at this boot, out of the modules the loader
//! happened to place** — and `docs/first-boot-outside-qemu.md` records the boot
//! where that order was not the order under QEMU. A supervisor carrying a
//! compiled-in list of what to spawn would be a supervisor that is right on one
//! machine.
//!
//! The handle is the same argument one layer in, and `f_abi::door::Entry`
//! already makes it: a second occupant of a place finds the same slot indices at
//! a later generation, so an index written down is an index that is eventually
//! somebody else's.
//!
//! # What is deliberately not here
//!
//! **No policy.** RFC 0008 says restart is the supervisor's act, and a decision
//! this page handed down would be the frame deciding and the supervisor typing.
//! What the frame writes here is *capability* — what may be spent, what may be
//! filled — and what the supervisor writes below [`at::SUBMITTED`] is what it
//! did. Between the two there is a decision, and this page is shaped so that the
//! decision has somewhere to be: a list of manifests and an account, rather than
//! an instruction. `kernel::component::policy::decide` is still in the frame and
//! `cargo xtask lint-owed` still says so.
//!
//! **No count of what went wrong.** The frame reads refusals off the ring it
//! answered them on; a second copy here would be a number the two sides could
//! disagree about, with no way to tell which was lying.
//!
//! # The second half is the component's, and the frame only reads it
//!
//! Offsets from [`at::SUBMITTED`] up are written by the *component* and read by
//! the frame after the run. That is `user/virtio-blk`'s arrangement and RFC
//! 0013's *read, never delivered*: the frame watching a component through memory
//! it granted, costing the component nothing and telling it nothing.
//!
//! A component that scribbles this half lies about its own counters and about
//! nothing else. It cannot lie about whether a place got filled — that is the
//! frame's own [`super`] place table, on the other side of the boundary — which
//! is what the boot actually turns on.

/// Where the frame maps this page in the component's address space.
///
/// The one number both sides hold. It must equal `kernel::process::BOARD`, and
/// the kernel asserts that at compile time rather than saying it in a comment —
/// `kernel/src/component.rs` holds the assertion, because the kernel is the
/// artefact that links both definitions and a comment is not a check.
///
/// The same address the three driver crates name, because a component's address
/// space is its own and one address per *shape* is what `f_ring::heap::AT`
/// settled. Same address, different layout, one assertion each.
///
/// Unit: bytes, in the component's own address space.
pub const AT: u64 = 0x0041_8000;

/// How many bytes the page is. One frame.
/// Unit: bytes.
pub const BYTES: u32 = 4096;

/// A word the frame writes last and the component checks before it believes
/// anything else here.
///
/// R04, at the one place this component reads a structure it did not build. A
/// page of zeroes is what an unmapped-and-then-mapped frame looks like, and a
/// supervisor that took a zero for an account handle would spend capability
/// zero — which resolves, because handle zero is a handle. The magic makes *the
/// frame did not fill this in* a distinct answer from *the frame said zero*, and
/// on this page that distinction is the difference between refusing and
/// spawning something nobody asked for.
pub const MAGIC: u64 = 0x7375_7065_725F_7276;

/// How many places this page can name.
///
/// Four, which is [`super::PLACES`] minus the supervisor's own — a supervisor
/// does not fill the place it is standing in, and a table with room for it
/// would be a table whose first entry has to be skipped by a rule rather than
/// absent by construction.
///
/// It bounds the page rather than the machine: a build with more places than
/// this fails [`Board::read`] with [`f_abi::error::resource::QUOTA_EXHAUSTED`]
/// rather than filling the ones that fit, because a supervisor that silently
/// supervised a prefix would report success over a place nobody was watching.
/// Unit: places.
pub const PLACES_MAX: usize = 4;

/// Byte offsets of the fields the frame writes, each a little-endian `u64`.
///
/// Slots rather than a `repr(C)` struct, because both sides read and write them
/// through `f_ring::device::Window`, which is a bounds-checked volatile accessor
/// and not a reference — there is no struct to borrow. Eight bytes each even
/// where four would do, so that adding a field never moves one.
pub mod at {
    /// [`super::MAGIC`]. Unit: none.
    pub const MAGIC: u32 = 0;
    /// Where this component's control ring is.
    /// Unit: bytes, in the component's address space.
    pub const CONTROL_AT: u32 = 8;
    /// How many bytes of it. Unit: bytes.
    pub const CONTROL_LEN: u32 = 16;
    /// The `Untyped` this supervisor may spend, as an `f_abi::door::Entry`
    /// handle.
    ///
    /// Told rather than derived from the spawn argument, even though the
    /// argument carries it today. The argument is one word with a selector in
    /// it; an account is a capability, and the day a supervisor holds two of
    /// them — one per tenant — is the day a field that was the argument's low
    /// half has to become a table. It is a table now, with one row.
    /// Unit: none — a capability handle.
    pub const ACCOUNT: u32 = 24;
    /// How many of [`MANIFEST`] are real. Unit: places.
    pub const PLACES: u32 = 32;
    /// The first place's manifest, by content hash — what
    /// `f_abi::control::op::SPAWN` carries in `ext[0]`.
    ///
    /// A content hash and not an index, because an index is a statement about
    /// the order the loader placed modules in and a hash is a statement about
    /// what the module *is*. The boot that swapped places 2 and 3 on hardware
    /// (`TODO.md` E0-P18) is why that distinction is load-bearing rather than
    /// fastidious.
    /// Unit: none — an `f_abi::ContentId`.
    pub const MANIFEST: u32 = 40;
    /// How far apart two [`MANIFEST`] rows are. Unit: bytes.
    pub const MANIFEST_STRIDE: u32 = 8;

    // --- what the component writes, and the frame reads afterwards ------------

    /// How many spawns this supervisor put on its control ring. Unit: entries.
    pub const SUBMITTED: u32 = 128;
    /// How many it could not, because the ring had no room. Unit: entries.
    ///
    /// **This is the only refusal this component can see**, and the module
    /// comment says why: nothing answers its ring until it has stopped running,
    /// so what the frame *made of* a submission is not knowable here. The frame
    /// prints what it filled, out of its own place table, beside this number.
    ///
    /// Not a packed error, because there is only one thing it can be — and a
    /// field typed as an error would invite a reader to expect the frame's
    /// refusals in it, which are on the other side of the boundary and stay
    /// there.
    /// Unit: entries.
    pub const REFUSED: u32 = 136;
}

/// What the frame wrote, read once and believed thereafter.
///
/// Copied out rather than re-read, which is `f_ring::Mapping`'s discipline and
/// the reason that file gives: validating a page in place and then acting on a
/// later read of it is a check that bounds nothing. Nothing else writes this
/// page while this component runs — the frame fills it before the first
/// instruction and reads the far half after the last one — but the copy costs
/// four words and does not depend on that staying true.
#[derive(Clone, Copy)]
pub struct Board {
    /// Where the control ring is. Unit: bytes, in this address space.
    pub control_at: u64,
    /// How many bytes of it. Unit: bytes.
    pub control_len: u32,
    /// The account this supervisor may spend.
    pub account: u32,
    /// The manifests it may put into places, in the frame's order.
    pub manifests: [u64; PLACES_MAX],
    /// How many of `manifests` are real. Unit: places.
    pub places: usize,
}

impl Board {
    /// Read the page the frame filled in.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/MALFORMED_HEADER` for a page whose magic is not
    /// [`MAGIC`] — which is the frame not having written it, and is refused
    /// before any other field is looked at.
    ///
    /// `RESOURCE/QUOTA_EXHAUSTED` for a page naming more places than
    /// [`PLACES_MAX`], so that a build which outgrew this page says so instead
    /// of supervising a prefix of itself.
    ///
    /// Whatever `f_ring::device::Window` refuses, for a page that is not there.
    pub fn read() -> Result<Self, i32> {
        let page = f_ring::device::Window::at(AT, BYTES)?;
        if page.read64(at::MAGIC)? != MAGIC {
            return Err(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::MALFORMED_HEADER,
            ));
        }
        let places = usize::try_from(page.read64(at::PLACES)?).unwrap_or(usize::MAX);
        if places > PLACES_MAX {
            return Err(f_abi::error::pack(
                f_abi::error::RESOURCE,
                f_abi::error::resource::QUOTA_EXHAUSTED,
            ));
        }
        let mut manifests = [0; PLACES_MAX];
        for (index, slot) in manifests.iter_mut().enumerate().take(places) {
            let offset = at::MANIFEST
                + u32::try_from(index)
                    .map_err(|_| {
                        f_abi::error::pack(
                            f_abi::error::RESOURCE,
                            f_abi::error::resource::QUOTA_EXHAUSTED,
                        )
                    })?
                    .saturating_mul(at::MANIFEST_STRIDE);
            *slot = page.read64(offset)?;
        }
        Ok(Self {
            control_at: page.read64(at::CONTROL_AT)?,
            control_len: u32::try_from(page.read64(at::CONTROL_LEN)?).unwrap_or(0),
            account: u32::try_from(page.read64(at::ACCOUNT)?).unwrap_or(0),
            manifests,
            places,
        })
    }

    /// Write back what this run did.
    ///
    /// Ignores its own refusals, and that is deliberate rather than lazy: this
    /// is the last thing the component does before [`f_abi::door::EXIT`], and a
    /// supervisor that died reporting would be a supervisor whose work the frame
    /// then could not see. The frame's own place table is what the boot checks;
    /// this is the component's account of itself beside it.
    pub fn report(submitted: u64, refused: u64) {
        let Ok(page) = f_ring::device::Window::at(AT, BYTES) else { return };
        let _ = page.write64(at::SUBMITTED, submitted);
        let _ = page.write64(at::REFUSED, refused);
    }
}
