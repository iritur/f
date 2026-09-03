// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What a driver component asks the frame for, and what it is refused with.
//!
//! # The seam this fills, and why it was left rather than invented here
//!
//! `f_ring::registry` has carried two traits since E1-B10 with nothing behind
//! them: [`Domains`], which a service calls to give a registered buffer a
//! translation, and [`PageWalk`], which it calls to ask whether a device
//! reaches an address in the submitter's own space. Both were written as seams
//! for this task, and both are implemented here rather than replaced. That is
//! worth stating because the alternative was live: an IOMMU has a wider
//! interface than two methods, and it would have been easy to define a second
//! one here and leave `registry`'s pointing at test doubles forever. Two
//! interfaces to one mechanism is how the untested one comes to be wrong, and
//! `registry`'s is the one the drivers call.
//!
//! What is *added* rather than replaced is [`Domain`] and [`Unit`] — the
//! objects a translation belongs to — because `registry`'s traits deliberately
//! say nothing about which domain a service is mapping into. A service holds
//! one; the trait is what it does with it.
//!
//! # A device address is a physical address here, and that is a decision
//!
//! [`Grant::map`] answers the physical address of the memory a capability
//! names. The device's address space is therefore the identity of the
//! machine's, per domain, and the isolation comes entirely from *which* pages
//! are present in that domain's tables rather than from where they appear.
//!
//! Two things follow and both are the point. A driver already knows the
//! physical address of memory it holds a `Frame` capability for —
//! `cap::Found::object` tells it, and `cap.rs` argues at length that a physical
//! address a component cannot map is not a secret — so nothing is disclosed by
//! this that was not already. And a domain with no allocator of its own has no
//! second structure that can disagree with the page tables about what is where.
//!
//! *Reversal:* a device that cannot address the whole of physical memory —
//! a 32-bit bus master on a machine with more than four gibibytes — where an
//! identity domain has addresses the device cannot form. The answer then is a
//! per-domain address allocator handing out low addresses, and it is a change
//! to this file and to nothing that calls it, which is the reason the address
//! is *answered* by [`Grant::map`] rather than assumed by its caller.
//!
//! # Refusals are the ring's, unchanged
//!
//! Everything here refuses with an [`f_abi::error`] pair, because a service
//! passes them straight into a completion and RFC 0010 says a refusal names its
//! domain. A refusal this module invented a code for is a refusal a client
//! cannot act on, so there are none: `AUTHORITY` for a capability not held, or
//! held without the rights a device mapping needs,
//! `RESOURCE`/`QUOTA_EXHAUSTED` for a domain with no room, and
//! `ARGUMENT`/`BAD_ADDRESS` for a length that is not a whole number of pages.
//!
//! # Where this is exercised
//!
//! `kernel/src/arch/x86_64/dma.rs` builds a real [`Table`], grants it `Frame`
//! capabilities for the pages the device is allowed, and reaches the remapping
//! unit through [`Grant`] rather than around it — so every `cargo xtask iommu`
//! boot runs the handle resolution, the rights check, the extent bound and the
//! walk in [`Grant::reaches`]. It also asks for a translation it is not
//! entitled to and requires the refusal. That matters more than it looks: an
//! interface three tasks are told to build on, that no boot has ever called,
//! is a design document with a type signature.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, Handle, rights};
use f_abi::error;
use f_ring::registry::{Domains, PageWalk, Refusal};

use crate::arch::x86_64::vtd::{self, Refuse};
use crate::cap::Table;
use crate::mem::{FRAME_SIZE, FrameAllocator};

pub use crate::arch::x86_64::vtd::{Domain, Fault, Faults, Unit};

/// Turn a refusal from the unit into one a client can act on.
///
/// The mapping is deliberately lossy in one direction and not the other: every
/// way of running out of room becomes `RESOURCE`/`QUOTA_EXHAUSTED`, because
/// that is the same thing from the client's point of view and RFC 0029 already
/// conflates a full table with an unaffordable one for the same reason. Every
/// way of naming the wrong place stays `ARGUMENT`/`BAD_ADDRESS`, because a
/// client that named the wrong place can name a different one and a client that
/// ran out of room cannot.
fn refusal(refuse: Refuse) -> Refusal {
    let (domain, code) = match refuse {
        Refuse::NoDomains | Refuse::TooManyTables | Refuse::NoFrames => {
            (error::RESOURCE, error::resource::QUOTA_EXHAUSTED)
        }
        Refuse::Overlap | Refuse::NotMapped => (error::ARGUMENT, error::argument::BAD_ADDRESS),
        // Everything else is the unit itself being unusable, which is not
        // something the client did and not something it can retry. `DEVICE`
        // is the domain RFC 0010 puts hardware failures in, and the detail is
        // the unit's own reason rather than a code invented here.
        _ => (error::DEVICE, 0),
    };
    (error::pack(domain, code), 0)
}

/// One component's authority to hand memory to a device.
///
/// Borrows everything it needs and owns nothing, which is what lets it be
/// created for the duration of one ring entry and dropped afterwards. The
/// alternative — a long-lived object holding the allocator — would be a second
/// owner of the frame allocator, and there is exactly one.
pub struct Grant<'a> {
    /// The remapping unit the component's device is behind.
    pub unit: &'a mut Unit,
    /// The component's own domain.
    pub domain: &'a mut Domain,
    /// Where frames are, and where its tables come from.
    pub frames: &'a mut FrameAllocator,
    /// The component's capability table, which is what a handle is resolved
    /// against. Shared rather than mutable: giving a device a translation
    /// changes the page tables and never the capability.
    pub table: &'a Table,
}

impl Grant<'_> {
    /// How many whole pages `len` bytes span, refusing zero and refusing a
    /// length that is not a whole number of them.
    ///
    /// Refusing rather than rounding up, and this is the one place in the file
    /// where the reasoning is worth more than the code: a length rounded up to
    /// a page is a device given access to whatever follows the buffer, which is
    /// the exact failure the whole module exists to prevent, arrived at by
    /// being helpful. R04.
    fn pages(len: u32) -> Result<u64, Refusal> {
        let len = u64::from(len);
        if len == 0 || !len.is_multiple_of(FRAME_SIZE) {
            return Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), len));
        }
        Ok(len / FRAME_SIZE)
    }
}

impl Domains for Grant<'_> {
    /// Give this component's domain a translation for `len` bytes of the memory
    /// `cap` names.
    ///
    /// The capability must be a `Frame` carrying `READ` **and** `GRANT`, and
    /// `WRITE` decides whether the device may write through the translation —
    /// so a component that holds a read-only frame gets a read-only device
    /// mapping, and a device that writes to it faults. That is the rights
    /// bitmap being enforced rather than recorded, which is the same argument
    /// `paging::UserPage::ReadOnly` makes about the processor's tables: a right
    /// the mapping cannot express is a right that is not enforced.
    ///
    /// # Why `GRANT` and not `READ` alone
    ///
    /// Because a translation is a transfer. `abi::cap::rights::GRANT` is *may
    /// be transferred to another component*, and withholding it is how a
    /// component is handed authority it cannot pass on in any form — and a bus
    /// master is the one recipient in this system that is not a component at
    /// all. Once a page is in a device's domain, the device reaches it on its
    /// own schedule, through a mechanism the capability system does not
    /// mediate; the frame cannot take that back except by unmapping, which is
    /// exactly the shape of a transfer rather than of a read. So a component
    /// holding a `Frame` with `READ` and no `GRANT` — memory it may use and may
    /// not pass on — is refused here, and the refusal is `AUTHORITY` /
    /// `RIGHT_NOT_HELD`, which is what [`Domains::map`]'s own contract says it
    /// will be.
    ///
    /// This was `READ` alone until the implementation was read against the
    /// trait it implements. It is written down because a reader who thinks a
    /// device mapping is not a transfer would change it back, and the argument
    /// against that is the paragraph above rather than this line of code.
    fn map(&mut self, cap: u32, len: u32) -> Result<u64, Refusal> {
        let pages = Self::pages(len)?;
        let handle = Handle::from_bits(cap);
        let found = self
            .table
            .invoke(handle, CapType::Frame, rights::READ | rights::GRANT)
            .map_err(|packed| (packed, u64::from(cap)))?;

        // The capability's own extent bounds the grant. A component asking for
        // more than the frame it holds is asking for its neighbour's memory,
        // and the answer is a refusal rather than a shorter mapping — a partial
        // success here would be a device with a translation for part of what its
        // driver believes it has.
        let asked = pages.saturating_mul(FRAME_SIZE);
        let held = if found.extent == 0 { FRAME_SIZE } else { found.extent };
        if asked > held {
            return Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), asked));
        }

        let writable = rights::holds(found.rights, rights::WRITE);
        let base = found.object;
        for page in 0..pages {
            let at = base.saturating_add(page.saturating_mul(FRAME_SIZE));
            // SAFETY: `frames` is rebound onto the direct map of the active
            // address space — the caller of the ring holds it that way for the
            // whole of a boot — and `at` names memory the component holds a
            // capability for, which is the entitlement `invoke` just checked.
            let mapped = unsafe { self.unit.map(self.frames, self.domain, at, at, writable) };
            if let Err(refuse) = mapped {
                // Everything already mapped for this request is taken back
                // before the refusal is returned. A half-mapped grant would be
                // a device that reaches the first half of a buffer its driver
                // was told it does not have at all.
                for done in 0..page {
                    let undo = base.saturating_add(done.saturating_mul(FRAME_SIZE));
                    // SAFETY: as above, over a translation this loop just made.
                    let _ = unsafe { self.unit.unmap(self.frames, self.domain, undo) };
                }
                return Err(refusal(refuse));
            }
        }

        Ok(base)
    }

    /// Take the translation away again.
    ///
    /// Cannot refuse, which is what [`Domains`] requires and is not a weakening:
    /// RFC 0008 revokes a dead component's buffer sets whether or not anything
    /// is convenient, and a teardown a peer could decline is a peer that keeps
    /// its device's access by declining. A page that was not mapped is skipped
    /// rather than reported, because the only caller that can produce one is a
    /// second unmap of the same set — and the second one has nothing to do.
    fn unmap(&mut self, _cap: u32, address: u64, len: u32) {
        let Ok(pages) = Self::pages(len) else { return };
        for page in 0..pages {
            let at = address.saturating_add(page.saturating_mul(FRAME_SIZE));
            // SAFETY: as `map`: the allocator is rebound onto the direct map,
            // and this walks tables this module made.
            let _ = unsafe { self.unit.unmap(self.frames, self.domain, at) };
        }
    }
}

impl PageWalk for Grant<'_> {
    /// Does the device reach `len` bytes at `address`?
    ///
    /// Answered by walking the domain's second-level tables, which is the same
    /// walk the unit performs — so a *yes* here is the unit's own answer rather
    /// than a record of what this kernel believes it programmed.
    ///
    /// The honest qualification `registry` already states applies unchanged:
    /// nothing this project can boot walks a *component's* page tables, because
    /// QEMU's virtio offers no address translation services. This answers for
    /// the domain's tables, which is the registered path's question asked in the
    /// virtual path's shape. A machine with real address-translation services
    /// would answer it against the component's own tables and this would be
    /// wrong — so it is written down here rather than discovered there.
    fn reaches(&self, address: u64, len: u32) -> bool {
        // SAFETY: the allocator is rebound onto the direct map of the active
        // address space, which is what makes the domain's tables readable.
        unsafe { self.unit.reaches(self.frames, self.domain, address, u64::from(len)) }
    }
}

/// What the boot path found, for the log and for the state tree.
#[derive(Clone, Copy, Debug)]
pub struct Found {
    /// How many address bits a translation covers. Unit: bits.
    pub width: u8,
    /// How many levels a second-level walk has.
    pub levels: u8,
    /// How many domain ids the unit has.
    pub domains: u32,
    /// Whether the unit caches not-present entries.
    pub caching_mode: bool,
    /// Whether its page walks snoop the processor's caches. False means this
    /// kernel flushes every table entry it writes by hand, which is invisible
    /// under emulation and is why it is published.
    pub coherent: bool,
    /// The unit's capability register, so a reader can check the decisions
    /// above against what the machine actually said.
    pub capability: u64,
    /// Its extended capability register.
    pub extended: u64,
}

impl Found {
    /// Read the unit's own description of itself.
    #[must_use]
    pub fn of(unit: &vtd::Unit) -> Self {
        Self {
            width: unit.width(),
            levels: unit.levels(),
            domains: unit.domains(),
            caching_mode: unit.caching_mode(),
            coherent: unit.coherent(),
            capability: unit.capability(),
            extended: unit.extended(),
        }
    }
}
