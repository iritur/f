// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The four names `kernel/src/cap.rs` takes from `kernel/src/mem.rs`, and
//! nothing else.
//!
//! # Why a stand-in rather than the real allocator
//!
//! Because the checker's unit of work is a crate, and the real `mem` reaches
//! the page tables, the direct map and `x86_64` assembly on the way to a
//! frame. Handing all of that to a bounded model checker would not be a
//! stronger proof of the capability properties — it would be a proof about a
//! buddy allocator that never finished.
//!
//! What the table actually asks of `mem` is small enough to write down and
//! therefore small enough to argue about: a page size, a physical address it
//! never dereferences, and — for `properties::self_test`, which the proofs do
//! not run — somewhere to get two frames. Everything the five properties are
//! *about* is in `cap.rs` and is compiled here from that file rather than
//! copied.
//!
//! The honest gap, stated because it is the whole cost of this arrangement:
//! the proofs say nothing about `Direct`, which is the one `Backing` a running
//! process uses. `Direct::reach` is three lines and the boot suite exercises
//! it on every boot; the properties below are about what the table does with
//! whatever address a backing answers with. See RFC 0053.

/// How many bytes a table buys at a time.
///
/// **Not 4096, and this is the bound the growing proofs are stated inside.**
/// `SLOTS_PER_PAGE` is `FRAME_SIZE / size_of::<Slot>()`, and every loop in
/// `cap.rs` runs to `TABLE_SLOTS + grown * SLOTS_PER_PAGE`. At the kernel's
/// page size a table that has bought one page is 160 slots and the revocation
/// walk is 160 x 161 iterations; a bounded model checker unrolls a loop rather
/// than summarising it, so that is twenty-six thousand copies of the body with
/// symbolic slots in each. At 256 bytes a page is eight slots, a bought table
/// is forty, and the walk is a loop the checker finishes.
///
/// # What the reduction binds, which is less than it looks
///
/// `FRAME_SIZE` reaches `cap.rs` in exactly two places: the arithmetic
/// `Table::retype` performs on an untyped watermark, and the page
/// `Table::grow` buys. **A harness that never grows its table therefore cannot
/// depend on this value at all** — and nine of the ten never do.
///
/// That is an argument, so it is not left as one. `cargo xtask prove` runs
/// those nine a second time under [`wide-page`](self), which is the kernel's
/// own 4096, and requires them to verify both ways. If the independence is real
/// they pass twice; if it is not, the second pass is where that is found rather
/// than reasoned about.
///
/// So the honest statement of the bound is: `total_bought` is proved for a page
/// of eight slots and not for a page of a hundred and twenty-eight. A defect
/// that only appears at the larger page — an index computation that eight
/// cannot overflow — is outside it. It is inside `cap::properties::self_test`,
/// which runs the same code at the real size on every boot, and the two
/// instruments are not substitutes for each other. RFC 0053.
///
/// Unit: bytes. Epoch: none. Zero: not meaningful — a page is never empty.
#[cfg(not(feature = "wide-page"))]
pub const FRAME_SIZE: u64 = 256;

/// The kernel's own page size, for the harnesses whose cost does not depend on
/// it.
///
/// Turned on by `cargo xtask prove`'s second pass. See the constant above for
/// what running it twice is evidence of; the short version is that it turns
/// *the harnesses that never grow a table cannot depend on the page size* from
/// a claim about the code into a check that fails if it stops being true. The
/// count is `prove`'s to print — it derives it from `PROOF_HARNESSES` — and is
/// deliberately not written here, because a count in prose beside a count a
/// command computes is a count that goes stale. Unit: bytes.
#[cfg(feature = "wide-page")]
pub const FRAME_SIZE: u64 = 4096;

/// One order of the buddy allocator. Only [`Order::FRAME`] is ever named here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Order(u8);

impl Order {
    /// A single frame.
    pub const FRAME: Self = Self(0);
}

/// A run of physical frames, named the way the real one is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame {
    base: u64,
    order: Order,
}

impl Frame {
    /// The physical address of the first byte.
    #[must_use]
    pub const fn addr(self) -> u64 {
        self.base
    }

    /// Name a single frame by its physical address.
    #[must_use]
    pub const fn from_addr(addr: u64) -> Self {
        Self { base: addr, order: Order::FRAME }
    }
}

/// Enough of the allocator for `cap.rs` to compile against.
///
/// It hands out nothing. `properties::self_test` is the only caller of
/// `alloc_zeroed` and `free`, and no proof runs it: the boot does, which is
/// where a suite that needs real frames belongs. A proof harness builds its
/// table through [`crate::pages::Pages`] instead, which is a `Backing` whose
/// memory the harness owns for the length of the harness.
#[derive(Default)]
pub struct FrameAllocator {
    _private: (),
}

impl FrameAllocator {
    /// Where a frame can be read and written.
    ///
    /// The identity, because nothing in this crate has a direct map: the only
    /// addresses that reach here are the ones a harness handed to a table as
    /// an account's object, and a harness never lets the table dereference
    /// one.
    #[must_use]
    pub fn virt(&self, frame: Frame) -> *mut u8 {
        frame.addr() as *mut u8
    }

    /// Refuses. See the type comment.
    #[must_use]
    pub fn alloc_zeroed(&mut self, _order: Order) -> Option<Frame> {
        None
    }

    /// Refuses, and cannot be reached: nothing here allocates.
    ///
    /// # Safety
    ///
    /// `frame` must have come from [`Self::alloc_zeroed`], which never
    /// answers, so this has no legal caller. It is `unsafe` because the real
    /// one is, and a stand-in whose signature differs would let `cap.rs`
    /// compile here and not there.
    pub unsafe fn free(&mut self, _frame: Frame) {}
}
