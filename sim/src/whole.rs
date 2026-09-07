// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The whole system as one hash, and the descent that names where two of them
//! stop agreeing.
//!
//! # What a whole-system state is here
//!
//! Every component's published RFC 0013 tree, read at the end of a run — when
//! virtual time has stopped and nothing is in flight. That qualification is not
//! a convenience of the simulator, it is RFC 0013's own condition: a snapshot is
//! atomic *per node* and not across the tree, so a reading that spans two
//! components means something only where the system is quiesced. The RFC says
//! so in as many words — *E2's two-hash comparison is a comparison of two
//! quiesced trees, and that is the configuration in which it is meaningful* —
//! and [`crate::Outcome::trees`] is that configuration.
//!
//! # Why this is a second hash and not `Region::snapshot`
//!
//! `f_abi::state::snapshot` is one region's hash and it is defined over bytes,
//! deliberately, so that a reader too old to name half the nodes still computes
//! what a newer one computes. That argument is about *one region read by two
//! readers of different ages*.
//!
//! This asks a different question — *are these two runs of one commit in the
//! same state* — where the two readers are the same build and the answer that
//! must not be missed is a component publishing a different tree. So the fold
//! here names what it hashed: the component's key, its node count, and each
//! node's id, kind and unit beside its word. Two runs of one commit have one
//! schema, so the description costs nothing and buys the corner where a word
//! stood still while the tree around it moved.
//!
//! *What would reverse this:* somebody comparing whole-system roots across two
//! **builds**, where the schemas legitimately differ and folding them in would
//! make every root disagree for a reason that is not a state. At that point the
//! description comes out of the fold and [`Divergence::Shape`] becomes the only
//! thing that reports it — which is a worse answer than this one, and is why the
//! condition is written down rather than the choice being hedged.
//!
//! # Why the descent exists at all
//!
//! Because *two roots differed* is not a finding anybody can act on, and a check
//! that reports one is a check whose output is a second investigation — by a
//! person, reading two logs. `generation/src/diff.rs` makes exactly this
//! argument one layer up about two record trees, and [`Divergence`] is that
//! argument's shape reused: a name, and the cases a name cannot carry.
//!
//! # Determinism
//!
//! A `BTreeMap` keyed by the component's name, and a descent that walks it in
//! key order and each tree in schema order — which `f_abi::state::validate` has
//! already required to be ascending by node id. Nothing here draws, reads a
//! clock, or iterates a hash map. The one thing that *does* draw is
//! [`Whole::pick`], and it draws through `f_env::split` at a named identity —
//! RFC 0026 — so that which node a harness disturbs is a function of the seed
//! and of nothing else.

use std::collections::BTreeMap;

use f_abi::state::{Region, SchemaEntry, kind, write_word};
use f_hash::Sha256;

use crate::Outcome;

/// What every hash in this module is prefixed with.
///
/// A domain tag, so that a whole-system root cannot collide with any other
/// SHA-256 this tree computes over similar bytes — a blob's content address, a
/// generation leaf. The version in it is this module's fold, and it moves when
/// the fold does: a root recorded under one fold and compared against another
/// would disagree for a reason that is not a state, and a tag that never moved
/// would make that failure silent.
/// Unit: none — a constant.
pub const FOLD_TAG: &[u8] = b"f whole-system state v1\n";

/// The identity [`Whole::pick`] draws at.
///
/// A named site under RFC 0026: the label keys a stream of its own, so the node
/// a harness disturbs does not depend on how many values anything else drew
/// first. Unit: none — a label.
pub const PICK_SITE: &str = "compare.node";

/// Which of two states a finding is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The first state given to the comparison.
    Left,
    /// The second.
    Right,
}

impl Side {
    /// A word for a report.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Where two whole-system states stop agreeing.
///
/// Ordered by the descent that produces it — components in key order, nodes in
/// schema order, and the root last — so a caller printing these in the order the
/// enum declares them is printing them in the order a reader should read them.
///
/// Nothing here decides which side is right. A comparison of run *a* with run
/// *b* has no basis for calling either one the deviation, and a variant that
/// named one would be inviting its caller to go and fix the wrong run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Divergence {
    /// Both roots agree, and so does every subtree under them.
    Agree,
    /// One side publishes a component the other does not.
    ///
    /// Not a node divergence, because there is no pair to compare: a component
    /// that ran on one side and not the other is a difference in the *system*
    /// and not in a number it kept.
    Absent {
        /// The component's key, `"<actor>/<index>"`. Unit: none — a name.
        component: String,
        /// Which side has it. Unit: none.
        present: Side,
    },
    /// Both sides publish it and one of them published bytes that are not a
    /// tree.
    ///
    /// A finding about the publisher rather than about the state, and named
    /// rather than folded into the hash, because *this component cannot be
    /// read* and *this component has done nothing* are the two answers RFC 0013
    /// exists to keep apart.
    Unreadable {
        /// The component's key. Unit: none — a name.
        component: String,
        /// The side whose bytes would not open. Unit: none.
        side: Side,
    },
    /// Both sides publish it, both are readable, and the two are not the same
    /// tree.
    ///
    /// A schema moved. Reported by looking rather than inferred from the hash,
    /// which is the division this whole module rests on: the hash says *these
    /// differ*, and the descent is what interprets.
    Shape {
        /// The component's key. Unit: none — a name.
        component: String,
        /// How many nodes the left side declares. Unit: nodes.
        left_nodes: u32,
        /// How many the right side declares. Unit: nodes.
        right_nodes: u32,
        /// The first schema position whose description differs, or `None` when
        /// only the counts do. Unit: index into the data block, zero-based.
        at: Option<usize>,
    },
    /// One node's word differs. **This is the localisation the whole module is
    /// for**, and the only variant that names a subtree.
    Node {
        /// The component's key. Unit: none — a name.
        component: String,
        /// The path of the nearest enclosing subtree, component key first —
        /// `"app/0/app"`. This is the answer to *where did the two runs
        /// diverge*. Unit: none — a path.
        subtree: String,
        /// The path of the node itself, `"app/0/app/reclaimed"`.
        /// Unit: none — a path.
        path: String,
        /// The node's permanent id, which is what makes two readings across
        /// time comparable when the names have moved. Unit: none — an
        /// identifier.
        id: u32,
        /// The word the left side published. Unit: the node's own, which its
        /// schema entry states.
        left: u64,
        /// The word the right side published. Unit: as above.
        right: u64,
    },
    /// Every component agrees and the two roots do not.
    ///
    /// Nothing was fed two different states, so the fold itself is what
    /// differed. That is a defect in this module and not a divergence between
    /// two runs, and it is a variant rather than an `unreachable!()` for
    /// `f_generation::diff::Divergence::Compiler`'s reason: a panic inside a
    /// comparison is a worse answer than a name.
    ///
    /// It is also why this enum is not `Option<Node>`. That signature cannot
    /// express two roots differing with every node standing still, so a `None`
    /// there would report *these two runs agree* about a pair that demonstrably
    /// does not — the one wrong answer a comparison must never give.
    Root,
}

impl Divergence {
    /// The subtree this names, or `None` for a finding that names none.
    ///
    /// [`Divergence::Node`] names a subtree inside a component;
    /// [`Divergence::Absent`], [`Divergence::Unreadable`] and
    /// [`Divergence::Shape`] name the component and can name nothing finer,
    /// because what differs is the tree rather than a number in it.
    #[must_use]
    pub fn named(&self) -> Option<&str> {
        match self {
            Self::Agree | Self::Root => None,
            Self::Absent { component, .. }
            | Self::Unreadable { component, .. }
            | Self::Shape { component, .. } => Some(component),
            Self::Node { subtree, .. } => Some(subtree),
        }
    }

    /// The component this is about, or `None` for a finding about neither.
    #[must_use]
    pub fn component(&self) -> Option<&str> {
        match self {
            Self::Agree | Self::Root => None,
            Self::Absent { component, .. }
            | Self::Unreadable { component, .. }
            | Self::Shape { component, .. }
            | Self::Node { component, .. } => Some(component),
        }
    }

    /// One line, for a report a machine produced and a person may read.
    ///
    /// The exit this serves is *localised to a named subtree automatically,
    /// with no human reading a log* — so the name comes first and the numbers
    /// after it, and there is no case in which a reader has to go and diff
    /// something themselves to learn where the two runs parted.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Agree => "the two states agree".to_string(),
            Self::Absent { component, present } => {
                format!("{component} — published on the {} only", present.label())
            }
            Self::Unreadable { component, side } => format!(
                "{component} — the {} side published bytes that are not a tree",
                side.label()
            ),
            Self::Shape { component, left_nodes, right_nodes, at } => match at {
                Some(index) => format!(
                    "{component} — the two trees describe node {index} differently \
                     ({left_nodes} node(s) either side)"
                ),
                None => format!(
                    "{component} — {left_nodes} node(s) on the left, {right_nodes} on the right"
                ),
            },
            Self::Node { subtree, path, id, left, right, .. } => format!(
                "{subtree} — {path} (id {id}) is {left} on the left and {right} on the right"
            ),
            Self::Root => "the two roots differ and every component agrees — see \
                           `Divergence::Root`, which is about this fold and not about a run"
                .to_string(),
        }
    }
}

/// What [`Whole::disturb`] disturbed.
///
/// Returned rather than assumed, because the whole point of the injection is
/// that **the harness knows the answer it is asking for**: a test that bumped a
/// word and then went looking for whichever node moved would be checking that
/// the descent found *something*, which is what a hash already says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// The component's key. Unit: none — a name.
    pub component: String,
    /// Where in the data block. Unit: index, zero-based.
    pub index: usize,
    /// The node's permanent id. Unit: none — an identifier.
    pub id: u32,
    /// The path of the nearest enclosing subtree. Unit: none — a path.
    pub subtree: String,
    /// The path of the node itself. Unit: none — a path.
    pub path: String,
}

/// Every component's published tree, at one quiesced instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Whole {
    parts: BTreeMap<String, Vec<u8>>,
}

impl Whole {
    /// The whole-system state a finished run left behind.
    #[must_use]
    pub fn of(outcome: &Outcome) -> Self {
        Self { parts: outcome.trees.clone() }
    }

    /// The same, from parts a caller assembled itself.
    #[must_use]
    pub const fn from_parts(parts: BTreeMap<String, Vec<u8>>) -> Self {
        Self { parts }
    }

    /// How many components published. Unit: components.
    #[must_use]
    pub fn parts(&self) -> usize {
        self.parts.len()
    }

    /// Every component's key, in the order the descent walks them.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }

    /// One component's published bytes.
    #[must_use]
    pub fn part(&self, component: &str) -> Option<&[u8]> {
        self.parts.get(component).map(Vec::as_slice)
    }

    /// One component's subtree hash. Unit: none — a hash.
    #[must_use]
    pub fn subtree(&self, component: &str) -> Option<[u8; 32]> {
        self.parts.get(component).map(|bytes| fold_part(component, bytes))
    }

    /// The whole system as one hash. Unit: none — a hash.
    ///
    /// The component count goes into the fold before the parts do, so a system
    /// that lost a component whose subtree hashed to nothing cannot collide
    /// with one that never had it.
    #[must_use]
    pub fn root(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(FOLD_TAG);
        hash.update(&(self.parts.len() as u64).to_le_bytes());
        for (name, bytes) in &self.parts {
            hash.update(&fold_part(name, bytes));
        }
        hash.finish()
    }

    /// Where these two states first stop agreeing.
    ///
    /// Leaves before roots: every component is compared before the roots are,
    /// so a red comparison names the component that moved rather than reporting
    /// that two systems differed. [`Divergence::Root`] is reached only when
    /// nothing below it differs, which makes it a statement about this fold.
    #[must_use]
    pub fn diff(&self, other: &Self) -> Divergence {
        match self.divergences(other).into_iter().next() {
            Some(found) => found,
            None if self.root() == other.root() => Divergence::Agree,
            None => Divergence::Root,
        }
    }

    /// Every component that differs, in key order.
    ///
    /// Beside [`Whole::diff`] rather than instead of it, because a report that
    /// named one subtree while five differed would be overclaiming: *localised*
    /// means the answer is a name, not that there is only ever one. The caller
    /// prints the first and the count.
    #[must_use]
    pub fn divergences(&self, other: &Self) -> Vec<Divergence> {
        let mut names: Vec<&str> = self.names().collect();
        for name in other.names() {
            if !self.parts.contains_key(name) {
                names.push(name);
            }
        }
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .filter_map(|name| compare_part(name, self.part(name), other.part(name)))
            .collect()
    }

    /// Which component and node a seed names, at [`PICK_SITE`].
    ///
    /// RFC 0026's derivation, applied twice: once by the site's label and once
    /// by the occurrence, and then once more per field so that the component and
    /// the node are two independent identities rather than two halves of one
    /// draw. `None` for a state with no component in it, or whose named
    /// component publishes no node — both of which are systems there is nothing
    /// to disturb in, and neither is an error.
    #[must_use]
    pub fn pick(&self, seed: u64, occurrence: u64) -> Option<(String, usize)> {
        use f_env::split::{derive, label};
        let at = derive(derive(seed, label(PICK_SITE)), occurrence);
        let names: Vec<&str> = self.names().collect();
        let component = *names.get((derive(at, 0) % names.len().max(1) as u64) as usize)?;
        let region = Region::open(self.part(component)?).ok()?;
        let nodes = u64::from(region.nodes());
        if nodes == 0 {
            return None;
        }
        Some((component.to_string(), (derive(at, 1) % nodes) as usize))
    }

    /// Add one to a node's word, and answer what was disturbed.
    ///
    /// The injection E2-P05's exit is written against, and it is here rather
    /// than in a test for the reason `crate::state::Published` gives about its
    /// own bytes: a test that reached into the region and wrote a word would be
    /// a second writer of the format, and the format has one — the word goes
    /// through `f_abi::state::write_word`, which is the publisher's half of the
    /// same module the reader uses.
    ///
    /// Adding one rather than setting a value, because a set could land on the
    /// number that was already there and inject nothing, and an injection that
    /// silently did not happen is the one thing a test of a comparison must not
    /// contain. Wrapping rather than saturating for the same reason: a word at
    /// `u64::MAX` must still move.
    ///
    /// `None` for a component this state does not carry, a node it does not
    /// declare, or bytes that are not a tree.
    pub fn disturb(&mut self, component: &str, index: usize) -> Option<Site> {
        let site = self.site(component, index)?;
        let bytes = self.parts.get_mut(component)?;
        let word = Region::open(bytes).ok()?.word(index)?;
        write_word(bytes, index, word.wrapping_add(1)).ok()?;
        Some(site)
    }

    /// What lives at that position, named the way the descent would name it.
    #[must_use]
    pub fn site(&self, component: &str, index: usize) -> Option<Site> {
        let region = Region::open(self.part(component)?).ok()?;
        let entry = region.entry(index)?;
        let (subtree, path) = paths(component, &region, index);
        Some(Site { component: component.to_string(), index, id: entry.id, subtree, path })
    }
}

/// One component's subtree hash.
///
/// The description goes in beside the words — the module comment argues why, and
/// names the condition that would take it back out. The first byte says whether
/// what follows is a tree or bytes that are not one, so that a publisher who
/// wrote rubbish cannot collide with one who published nodes.
fn fold_part(name: &str, bytes: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(FOLD_TAG);
    hash.update(name.as_bytes());
    hash.update(&[0]);
    match Region::open(bytes) {
        Ok(region) => {
            hash.update(&[0]);
            hash.update(&region.nodes().to_le_bytes());
            for index in 0..region.nodes() as usize {
                // `nodes()` is what `open` validated, so every index below it
                // has an entry and a word. `unwrap_or` rather than an
                // expectation because a hash is not the place to end a run: a
                // component whose region went missing under the reader is a
                // divergence, and the descent is what says so by name.
                let entry = region.entry(index).unwrap_or(SchemaEntry::ZERO);
                hash.update(&entry.id.to_le_bytes());
                hash.update(&[entry.kind, entry.unit]);
                hash.update(&region.word(index).unwrap_or(0).to_le_bytes());
            }
        }
        Err(_) => {
            hash.update(&[1]);
            hash.update(bytes);
        }
    }
    hash.finish()
}

/// One component, either side, compared.
fn compare_part(name: &str, left: Option<&[u8]>, right: Option<&[u8]>) -> Option<Divergence> {
    let component = name.to_string();
    let (left, right) = match (left, right) {
        (Some(left), Some(right)) => (left, right),
        (Some(_), None) => return Some(Divergence::Absent { component, present: Side::Left }),
        (None, Some(_)) => return Some(Divergence::Absent { component, present: Side::Right }),
        // Unreachable: the name came from one of the two maps. Answered rather
        // than panicked for `Divergence::Root`'s reason.
        (None, None) => return None,
    };

    let (ours, theirs) = match (Region::open(left), Region::open(right)) {
        (Ok(ours), Ok(theirs)) => (ours, theirs),
        // Two publishers who both wrote rubbish and wrote the *same* rubbish are
        // in one state, and saying otherwise would make a comparison report a
        // divergence between a system and itself.
        (Err(_), Err(_)) if left == right => return None,
        (Err(_), _) => return Some(Divergence::Unreadable { component, side: Side::Left }),
        (_, Err(_)) => return Some(Divergence::Unreadable { component, side: Side::Right }),
    };

    if ours.nodes() != theirs.nodes() {
        return Some(Divergence::Shape {
            component,
            left_nodes: ours.nodes(),
            right_nodes: theirs.nodes(),
            at: None,
        });
    }

    for index in 0..ours.nodes() as usize {
        let same = match (ours.entry(index), theirs.entry(index)) {
            (Some(a), Some(b)) => (a.id, a.kind, a.unit) == (b.id, b.kind, b.unit),
            _ => false,
        };
        if !same {
            return Some(Divergence::Shape {
                component,
                left_nodes: ours.nodes(),
                right_nodes: theirs.nodes(),
                at: Some(index),
            });
        }
    }

    // The words, in schema order — which `f_abi::state::validate` has already
    // required to be ascending by id, so this is *the* order and not merely a
    // convenient one. The first that differs is the answer: one finding per
    // component, for the reason a compiler reports one error per expression.
    for index in 0..ours.nodes() as usize {
        let (a, b) = (ours.word(index).unwrap_or(0), theirs.word(index).unwrap_or(0));
        if a != b {
            let (subtree, path) = paths(name, &ours, index);
            let id = ours.entry(index).map_or(0, |entry| entry.id);
            return Some(Divergence::Node { component, subtree, path, id, left: a, right: b });
        }
    }
    None
}

/// The node's path, and the path of the subtree that encloses it.
///
/// Both begin with the component's key, because a node name is only unique
/// inside its own tree and a report that said `issued` would be naming a node in
/// whichever component the reader guessed.
fn paths(component: &str, region: &Region<'_>, index: usize) -> (String, String) {
    // Root-first, so a prefix of this chain is the path of an ancestor.
    let mut chain: Vec<SchemaEntry> = Vec::new();
    let mut cursor = region.entry(index);
    while let Some(entry) = cursor {
        chain.push(entry);
        if entry.parent == 0 {
            break;
        }
        // `validate` requires a parent to be a node named before its child, so
        // this walk is finite. The bound is here anyway, because this function
        // is handed regions from two runs and a bound costs one comparison.
        if chain.len() > region.nodes() as usize {
            break;
        }
        cursor = region.index_of(entry.parent).and_then(|at| region.entry(at));
    }
    chain.reverse();

    let join = |upto: usize| {
        let mut out = component.to_string();
        for entry in chain.iter().take(upto) {
            out.push('/');
            out.push_str(&String::from_utf8_lossy(entry.label()));
        }
        out
    };

    let path = join(chain.len());
    // The nearest enclosing subtree, counting the node itself: a subtree node
    // that moved *is* the subtree that diverged. A tree with no subtree node
    // above the divergence has the component itself as its answer, which is the
    // honest one — there is nothing finer to name.
    let subtree = chain
        .iter()
        .rposition(|entry| entry.kind == kind::SUBTREE)
        .map_or_else(|| component.to_string(), |at| join(at + 1));
    (subtree, path)
}

/// A hash as the lower-case hexadecimal a report prints and a caller compares.
#[must_use]
pub fn hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('?'));
        out.push(char::from_digit(u32::from(byte & 0xF), 16).unwrap_or('?'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Declared, Published};
    use f_abi::state::unit;

    /// A component with a subtree inside its root, so that *the subtree named*
    /// is a question with a non-trivial answer: a tree one level deep would
    /// make every localisation land on the component's own root, and the test
    /// would pass against a descent that never walked a parent chain.
    const APP: &[Declared] = &[
        Declared { id: 1, parent: 0, kind: kind::SUBTREE, unit: unit::NONE, name: b"app" },
        Declared { id: 2, parent: 1, kind: kind::SUBTREE, unit: unit::NONE, name: b"memory" },
        Declared { id: 3, parent: 2, kind: kind::GAUGE, unit: unit::FRAMES, name: b"free" },
        Declared { id: 4, parent: 1, kind: kind::COUNTER, unit: unit::ENTRIES, name: b"issued" },
    ];

    /// A second declaration, of a different width, for the shape case.
    const NARROW: &[Declared] = &[
        Declared { id: 1, parent: 0, kind: kind::SUBTREE, unit: unit::NONE, name: b"app" },
        Declared { id: 4, parent: 1, kind: kind::COUNTER, unit: unit::ENTRIES, name: b"issued" },
    ];

    /// A system of `count` components, each publishing [`APP`], with `base + n`
    /// in `issued` so that no two of them are byte-identical.
    fn system(count: usize, base: u64) -> Whole {
        let mut parts = BTreeMap::new();
        for index in 0..count {
            let mut published = Published::new(APP);
            published.set(4, base + index as u64);
            published.set(3, 7);
            parts.insert(format!("app/{index}"), published.bytes().to_vec());
        }
        Whole::from_parts(parts)
    }

    #[test]
    fn one_state_agrees_with_itself_and_the_descent_names_nothing() {
        // The control. Every assertion below would hold against a comparison
        // that reported a divergence for every pair it was ever given.
        let whole = system(3, 10);
        assert_eq!(whole.root(), whole.clone().root());
        assert_eq!(whole.diff(&whole), Divergence::Agree);
        assert_eq!(whole.diff(&whole).named(), None);
        assert!(whole.divergences(&whole).is_empty());
    }

    #[test]
    fn a_system_that_is_not_in_the_same_state_has_a_different_root() {
        // And the other half of the control: a root over something that does
        // not vary agrees with itself forever, which is indistinguishable from
        // a root that works. `cargo xtask trace` builds a deliberately broken
        // kernel to make this same point one layer down.
        assert_ne!(system(3, 10).root(), system(3, 11).root());
        assert_ne!(system(3, 10).root(), system(2, 10).root());
    }

    #[test]
    fn an_injected_divergence_is_named_and_not_merely_found() {
        // **E2-P05's exit, at the smallest scale it can be asked at.** The
        // harness knows the answer before it asks: `disturb` returns the site
        // it wrote, and the descent has to name that site rather than merely
        // report that two systems differed.
        let left = system(3, 10);
        let mut right = left.clone();
        let site = right.disturb("app/1", 2).expect("a node this test declared");

        assert_eq!(site.path, "app/1/app/memory/free");
        assert_eq!(site.subtree, "app/1/app/memory", "the enclosing subtree, not the component");

        assert_ne!(left.root(), right.root(), "a disturbed word did not reach the root");
        match left.diff(&right) {
            Divergence::Node { component, subtree, path, id, left: was, right: now } => {
                assert_eq!(component, "app/1");
                assert_eq!(subtree, site.subtree);
                assert_eq!(path, site.path);
                assert_eq!(id, 3);
                assert_eq!((was, now), (7, 8));
            }
            other => panic!("the descent answered {other:?} instead of naming the node"),
        }
        // And exactly one component diverged, which is what *localised* means:
        // a descent that named the right subtree while reporting that all three
        // had moved would be naming it by luck.
        assert_eq!(left.divergences(&right).len(), 1);
    }

    #[test]
    fn every_seeded_injection_is_named_exactly() {
        // The sweep, and the reason the injection is drawn rather than fixed: a
        // localisation checked at one node is a localisation checked at one
        // node. The site is picked through `f_env::split` at [`PICK_SITE`] —
        // RFC 0026's derive-by-identity — so which node is disturbed is a
        // function of the seed and the trial, and of nothing that ran before.
        let base = system(4, 100);
        let mut named = 0u32;
        for trial in 0..64u64 {
            let seed = 0xf00d_beef_cafe_1234u64;
            let (component, index) = base.pick(seed, trial).expect("a system with nodes in it");
            let mut hurt = base.clone();
            let site = hurt.disturb(&component, index).expect("a node `pick` named");
            match base.diff(&hurt) {
                Divergence::Node { component: which, subtree, path, id, .. } => {
                    assert_eq!(which, site.component, "trial {trial}: the wrong component");
                    assert_eq!(subtree, site.subtree, "trial {trial}: the wrong subtree");
                    assert_eq!(path, site.path, "trial {trial}: the wrong node");
                    assert_eq!(id, site.id, "trial {trial}: the wrong id");
                    named += 1;
                }
                other => panic!("trial {trial}: the descent answered {other:?}"),
            }
        }
        assert_eq!(named, 64, "every injection must be named, not most of them");
    }

    #[test]
    fn the_draw_is_a_function_of_the_seed_and_of_nothing_else() {
        // RFC 0004's contract, on this module's one draw. Two seeds must not
        // agree everywhere either, or the sweep above would be sixty-four
        // copies of one trial.
        let base = system(4, 100);
        let picks = |seed: u64| -> Vec<(String, usize)> {
            (0..32).filter_map(|trial| base.pick(seed, trial)).collect()
        };
        assert_eq!(picks(1), picks(1), "a seed must reproduce its picks exactly");
        assert_ne!(picks(1), picks(2), "two seeds picked the same nodes in the same order");
    }

    #[test]
    fn a_component_on_one_side_only_is_named_as_absent() {
        // Not a node divergence, because there is no pair to compare — and the
        // side is carried, because a comparison of run a with run b has no
        // basis for calling either one the deviation.
        let left = system(3, 10);
        let right = system(2, 10);
        assert_eq!(
            left.diff(&right),
            Divergence::Absent { component: "app/2".to_string(), present: Side::Left }
        );
        assert_eq!(
            right.diff(&left),
            Divergence::Absent { component: "app/2".to_string(), present: Side::Right }
        );
        assert_eq!(left.diff(&right).named(), Some("app/2"));
    }

    #[test]
    fn two_trees_of_different_shapes_are_named_as_shape() {
        // A component that republished a different tree. The hash would say
        // *these differ* and stop there; the descent is what says the schema
        // moved rather than a number inside it.
        let left = system(1, 10);
        let mut parts = BTreeMap::new();
        parts.insert("app/0".to_string(), Published::new(NARROW).bytes().to_vec());
        let right = Whole::from_parts(parts);
        assert_eq!(
            left.diff(&right),
            Divergence::Shape {
                component: "app/0".to_string(),
                left_nodes: 4,
                right_nodes: 2,
                at: None,
            }
        );
    }

    #[test]
    fn bytes_that_are_not_a_tree_are_named_rather_than_read_as_agreement() {
        // *This component cannot be read* and *this component has done nothing*
        // are the two answers RFC 0013 exists to keep apart, and a comparison
        // that folded the first into the second would erase the distinction at
        // the one moment it matters.
        let left = system(1, 10);
        let mut parts = BTreeMap::new();
        parts.insert("app/0".to_string(), vec![0u8; 4096]);
        let right = Whole::from_parts(parts);
        assert_eq!(
            left.diff(&right),
            Divergence::Unreadable { component: "app/0".to_string(), side: Side::Right }
        );
        // Two publishers who wrote the *same* rubbish are in one state, and a
        // comparison that reported otherwise would report a divergence between
        // a system and itself.
        assert_eq!(right.diff(&right), Divergence::Agree);
    }

    #[test]
    fn a_subtree_node_that_moved_is_its_own_subtree() {
        // The corner the `rposition` in `paths` exists for. A subtree node's
        // word is reserved and reads zero, so this only happens to a publisher
        // that wrote into one — and when it does, the thing that diverged is
        // that subtree and not its parent.
        let left = system(1, 10);
        let mut right = left.clone();
        let site = right.disturb("app/0", 1).expect("the memory subtree node");
        assert_eq!(site.path, "app/0/app/memory");
        assert_eq!(site.subtree, "app/0/app/memory");
        assert_eq!(left.diff(&right).named(), Some("app/0/app/memory"));
    }

    #[test]
    fn the_root_is_over_the_description_as_well_as_the_words() {
        // The module comment's decision, asserted rather than described: two
        // components publishing the same words under different declarations are
        // not in the same state. `f_abi::state::snapshot` deliberately answers
        // the other way, and the two hashes are for two different questions.
        let mut ours = BTreeMap::new();
        ours.insert("app/0".to_string(), Published::new(APP).bytes().to_vec());
        let mut theirs = BTreeMap::new();
        theirs.insert("app/0".to_string(), Published::new(NARROW).bytes().to_vec());
        assert_ne!(Whole::from_parts(ours).root(), Whole::from_parts(theirs).root());
    }

    #[test]
    fn a_name_is_part_of_the_state_and_not_only_a_label() {
        // The same words under two component keys are two different systems: a
        // fold that hashed only the regions would call a system whose client
        // moved places the system it was before.
        let mut ours = BTreeMap::new();
        ours.insert("app/0".to_string(), Published::new(APP).bytes().to_vec());
        let mut theirs = BTreeMap::new();
        theirs.insert("app/1".to_string(), Published::new(APP).bytes().to_vec());
        assert_ne!(Whole::from_parts(ours).root(), Whole::from_parts(theirs).root());
    }

    #[test]
    fn a_hash_is_printed_the_way_a_caller_compares_it() {
        let printed = hex(&[0u8; 32]);
        assert_eq!(printed.len(), 64);
        assert!(printed.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(hex(&system(1, 0).root()), printed, "a real root hashed to nothing");
    }
}
