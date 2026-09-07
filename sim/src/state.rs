// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What a modelled component publishes about itself: RFC 0013's tree, in a
//! buffer a host actor owns.
//!
//! # Why a simulated component publishes a real tree and not a summary
//!
//! Because the whole of RFC 0013's test of worth is *whether E1's fault sweeps
//! and E2's state comparison actually consume it*, and a fault sweep that
//! asserted against a struct of counters would be consuming a struct of
//! counters. What a scenario asserts here goes through `f_abi::state::Region` —
//! the same decoder the frame's `Reader` is checked against, over bytes in the
//! same format the frame writes into a component's frame — so a test that
//! passes is a test about the format a machine would publish, not about a
//! convenience this crate invented for itself.
//!
//! # The property that makes this affordable, and the one it costs
//!
//! **The region is where the counters live.** An actor does not keep a `u32`
//! and copy it in here; it has no `u32`. `Published::add` is the store the
//! actor was already making, which is RFC 0013's first property — *a map of
//! memory that already exists, not a serialisation of it* — and it is what
//! makes there be no collect step and no second copy that can disagree with the
//! first.
//!
//! What it costs is that the words are **state** and therefore have to travel
//! in a snapshot. [`Published::words`] and [`Published::restore`] are that, and
//! `snap.rs`'s own argument is why they must exist: a field that quietly did not
//! travel produces a plausible run that diverges from the real one, which is
//! worse than no snapshot at all. A tree published *from* fields that already
//! travelled would have avoided the work and would have been a projection
//! wearing this module's name.
//!
//! # Determinism
//!
//! Nothing here draws, reads a clock or iterates a hash map. A node is found by
//! a linear scan over a declaration that is a `const` slice, so two runs walk it
//! in one order because there is only one.

use f_abi::state::{Region, SchemaEntry, publish, write_word};

/// One node a modelled component declares.
///
/// The same five fields `[[state]]` carries in a manifest, and deliberately not
/// a `SchemaEntry`: the offset is derived from the position, so a declaration
/// that carried one would be a second opinion about where a word lives.
/// `f_abi::manifest::Node::entry` makes the same argument for the real
/// declaration, and this is that argument with no file in between.
#[derive(Clone, Copy, Debug)]
pub struct Declared {
    /// Permanent, never reused, ascending in declaration order. Unit: none.
    pub id: u32,
    /// The id this hangs under, or zero for the root. Unit: none.
    pub parent: u32,
    /// One of `f_abi::state::kind`. Unit: none.
    pub kind: u8,
    /// One of `f_abi::state::unit`. Unit: none — it *is* the unit.
    pub unit: u8,
    /// ASCII, at most sixteen bytes. Unit: none.
    pub name: &'static [u8],
}

/// How many bytes a published region is.
///
/// One page, which is what the frame charges a component's account for and is
/// therefore the bound a modelled component should be held to as well: a
/// simulator whose components could publish more than a machine's can is a
/// simulator that would not notice the day one did.
/// Unit: bytes.
pub const REGION_BYTES: usize = 4096;

/// A component's published state tree, and the memory its counters live in.
#[derive(Clone, Debug)]
pub struct Published {
    bytes: Vec<u8>,
    /// The declaration, so an id can be turned into an index without decoding
    /// the schema on every store. Unit: none.
    declared: &'static [Declared],
}

impl Published {
    /// Lay out a tree from a declaration.
    ///
    /// # Panics
    ///
    /// On a declaration `f_abi::state::publish` refuses, which after
    /// `f_abi::state::validate` means ids that do not ascend, a parent named
    /// after its child, or more nodes than a page holds. A `const` in this
    /// crate getting that wrong is a bug in this crate and not a run that
    /// should carry on — the same reading `Region::new` takes of a queue size
    /// the layout cannot hold.
    #[must_use]
    pub fn new(declared: &'static [Declared]) -> Self {
        let schema: Vec<SchemaEntry> = declared
            .iter()
            .enumerate()
            .map(|(index, node)| {
                SchemaEntry::new(
                    node.id,
                    node.parent,
                    index as u32 * f_abi::state::WORD,
                    node.kind,
                    node.unit,
                    node.name,
                )
            })
            .collect();
        let mut bytes = vec![0u8; REGION_BYTES];
        publish(&mut bytes, &schema).expect("a declaration this crate wrote is not a tree");
        Self { bytes, declared }
    }

    /// Where in the data block the node `id` lives.
    fn index_of(&self, id: u32) -> Option<usize> {
        self.declared.iter().position(|node| node.id == id)
    }

    /// The word the node `id` names. Zero for an id this tree does not carry,
    /// which is the reader's own answer to a node it cannot name.
    #[must_use]
    pub fn get(&self, id: u32) -> u64 {
        self.read().and_then(|region| region.value(id)).unwrap_or(0)
    }

    /// Put a value in the node `id` names. Does nothing for an id this tree
    /// does not carry, for the reason `kernel::state::Tree::set` does nothing:
    /// a publisher naming a node that does not exist is a bug in the publisher,
    /// and a panic would let a mistaken counter end a run that was about
    /// something else.
    pub fn set(&mut self, id: u32, value: u64) {
        let Some(index) = self.index_of(id) else { return };
        let _ = write_word(&mut self.bytes, index, value);
    }

    /// Add to it, saturating.
    ///
    /// Saturating rather than wrapping for `f_store::report::pack`'s reason: a
    /// counter that wrapped would report a small number for a large one, which
    /// is the shape of lie a counter exists not to tell.
    pub fn add(&mut self, id: u32, delta: u64) {
        self.set(id, self.get(id).saturating_add(delta));
    }

    /// The tree, decoded, or `None` for bytes that are not one.
    ///
    /// Through `f_abi::state::Region` and never through this type's own
    /// knowledge of its layout, which is the point: what a scenario reads is
    /// what a reader of a real component's mapping would read.
    #[must_use]
    pub fn read(&self) -> Option<Region<'_>> {
        Region::open(&self.bytes).ok()
    }

    /// The bytes, for a reader that wants to open them itself.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The snapshot hash of the tree as it stands. Unit: none — a hash.
    #[must_use]
    pub fn snapshot(&self) -> u64 {
        self.read().map_or(0, |region| region.snapshot())
    }

    /// Every word, in data-block order, for a snapshot file.
    #[must_use]
    pub fn words(&self) -> Vec<u64> {
        let Some(region) = self.read() else { return Vec::new() };
        (0..region.nodes() as usize).filter_map(|index| region.word(index)).collect()
    }

    /// Put them back.
    ///
    /// A count that is not this build's declaration is refused rather than
    /// padded: a file describing a tree with a different number of nodes is a
    /// file this build did not write, and restoring the words it *does*
    /// recognise would produce a plausible run whose counters are somebody
    /// else's. `snap.rs` spends a paragraph on why that is worse than refusing.
    #[must_use]
    pub fn restore(&mut self, words: &[u64]) -> bool {
        if words.len() != self.declared.len() {
            return false;
        }
        for (index, value) in words.iter().enumerate() {
            if write_word(&mut self.bytes, index, *value).is_err() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_abi::state::{kind, unit};

    const NODES: &[Declared] = &[
        Declared { id: 1, parent: 0, kind: kind::SUBTREE, unit: unit::NONE, name: b"app" },
        Declared { id: 2, parent: 1, kind: kind::COUNTER, unit: unit::ENTRIES, name: b"issued" },
        Declared { id: 3, parent: 1, kind: kind::GAUGE, unit: unit::ENTRIES, name: b"flight" },
    ];

    #[test]
    fn a_declaration_becomes_a_tree_a_reader_accepts() {
        let published = Published::new(NODES);
        let region = published.read().expect("bytes this module wrote were unreadable");
        assert_eq!(region.nodes(), 3);
        assert_eq!(region.entry(0).unwrap().label(), b"app");
        assert_eq!(region.entry(2).unwrap().unit, unit::ENTRIES);
        // Every word starts at zero, which is *this component has done nothing*
        // and not *this component cannot be read* — the distinction the whole
        // mechanism exists to make.
        assert_eq!(region.value(2), Some(0));
    }

    #[test]
    fn the_region_is_where_the_counters_live() {
        // The property the module comment claims: a store reaches the published
        // bytes and nothing else holds a copy, so the snapshot hash moves for
        // every increment and a reader sees the number the actor has.
        let mut published = Published::new(NODES);
        let before = published.snapshot();
        published.add(2, 1);
        assert_eq!(published.get(2), 1);
        assert_ne!(published.snapshot(), before, "a store did not reach the tree");

        published.set(3, 4);
        published.add(3, 1);
        assert_eq!(published.get(3), 5);

        // A node nothing declares is not a store somewhere else.
        published.set(99, 7);
        assert_eq!(published.get(99), 0);
    }

    #[test]
    fn every_word_travels_and_a_file_of_the_wrong_shape_is_refused() {
        let mut published = Published::new(NODES);
        published.add(2, 11);
        published.set(3, 2);
        let words = published.words();
        assert_eq!(words, vec![0, 11, 2]);

        let mut restored = Published::new(NODES);
        assert!(restored.restore(&words), "a file this crate wrote was refused");
        assert_eq!(restored.snapshot(), published.snapshot(), "a word did not travel");

        assert!(!restored.restore(&[1, 2]), "a file describing a different tree was accepted");
        assert_eq!(restored.snapshot(), published.snapshot(), "a refused restore wrote anyway");
    }
}
