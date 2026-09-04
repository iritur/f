// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Somewhere for a table to put the slots it buys.
//!
//! A table charges an `Untyped` capability and gets back a physical address;
//! `cap::Backing` is the step that turns that into somewhere writable, and
//! `cap::Direct` is the one a running process uses. This is the one a proof
//! uses, and it is a different implementation on purpose rather than for
//! convenience: `Direct` answers with the kernel's direct map, which does not
//! exist here, and a stand-in that pretended to be a direct map would be
//! proving something about an address arithmetic nobody runs.
//!
//! What the proofs are therefore about is stated once, here: **the table's
//! behaviour given a backing that keeps its promise.** Whether `Direct` keeps
//! it is a different question, answered by the boot suite on every boot.

use crate::cap::{Backing, MAX_PAGES};
use crate::mem::FRAME_SIZE;

/// How many `u64`s a page of slots is.
const WORDS_PER_PAGE: usize = FRAME_SIZE as usize / 8;

/// Every page a table could ever buy, owned by the harness.
///
/// Aligned by being an array of `u64`, which is the alignment a slot needs —
/// a slot's widest field is a `u64` and it has no larger one.
pub struct Pages {
    words: [u64; WORDS_PER_PAGE * MAX_PAGES],
    handed: usize,
}

impl Pages {
    /// Pages nobody has taken yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { words: [0; WORDS_PER_PAGE * MAX_PAGES], handed: 0 }
    }

    /// How many pages this backing has answered for.
    ///
    /// Asserted by the harnesses that grow a table, because a table that
    /// silently stopped growing would satisfy several of these properties by
    /// being smaller than the property is about.
    #[must_use]
    pub const fn handed(&self) -> usize {
        self.handed
    }
}

impl Default for Pages {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: each answer is the address of a distinct `WORDS_PER_PAGE`-word run
// inside `words`, which is `FRAME_SIZE` bytes and therefore exactly the
// `SLOTS_PER_PAGE` slots the trait asks for; the run is aligned to eight
// because `words` is, and a slot's alignment is eight. `handed` only ever
// advances, so no two answers overlap; nothing else in this crate reads
// `words`; and a `Pages` is declared before the `Table` it backs and dropped
// after it, so every address it has answered with is still live for as long as
// the table can reach it. That — the lifetime, not an exclusive borrow — is
// what makes the answers valid.
//
// Said plainly, because it is the obligation and not the mechanism: the `&mut
// self` here is *per call*, and the raw pointers the table keeps alias the same
// object across later calls. Under stacked borrows that is a violation Miri
// would flag; CBMC does not model aliasing, so no result in this crate rests on
// it either way. It is written down rather than argued away, because the honest
// version of this comment is the one that names the checker that would object.
//
// `MAX_PAGES` is the table's own ceiling, so the `None` below is unreachable
// rather than a refusal a proof depends on; it is written as a refusal anyway
// because a backing that panicked would be a harness that could fail a totality
// proof on its own account.
unsafe impl Backing for Pages {
    fn reach(&mut self, _phys: u64) -> Option<u64> {
        if self.handed >= MAX_PAGES {
            return None;
        }
        let base = self.words.as_mut_ptr().wrapping_add(self.handed * WORDS_PER_PAGE);
        self.handed += 1;
        Some(base as u64)
    }
}
