// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Which of the modules a loader offered is the one `f.root=` named.
//!
//! # Why the answer is a fold and not a filename
//!
//! `cargo xtask generation` names the file it packs `<root>.fcm`, and the
//! machine never sees that name: multiboot 1 hands the frame a base and a
//! length. So the only thing that can say *this is the generation you asked
//! for* is the fold over the records inside — the same arithmetic in the same
//! crate that produced the root on the build host, which is
//! `intent/0006-state/spec.md`'s sentence and the reason a machine is handed
//! records rather than a root alone.
//!
//! # Why this lives here rather than in the frame
//!
//! `abi/src/boot.rs` says the frame parses the token at `E2-P07`, and it does.
//! What it deliberately does not do is *decide* anything: the frame's whole
//! contribution is a loop over the modules it was given, and everything with a
//! branch in it is here — where `f-generation` is a `no_std` library the host
//! test harness can compile, and `kernel/` is a crate with `test = false` and
//! no host harness at all. A selection rule that could only be exercised by
//! booting QEMU is a selection rule whose refusals nothing checks.
//!
//! # What is and is not compared
//!
//! [`Examined::root`] is the fold over the record tree. It covers the leaves —
//! each component's *name* and the content address it is known by — and it does
//! not cover the component files themselves, which travel in the same module
//! and are not hashed by any fold. So two modules can carry one root and differ
//! in bytes: put a correct tree beside a file that is not the one its leaf
//! names, and every root comparison in this system passes.
//!
//! That is exactly what `E2-P07`'s *bit-identical* clause is about, so this
//! type carries [`Examined::digest`] beside the root: the SHA-256 of the whole
//! module as the loader delivered it, tree and files and framing together. A
//! root that matches is necessary; a digest that matches is the whole of what
//! arrived. The two are reported separately rather than folded into one number
//! because a run in which they disagree is a run that has found something, and
//! a single number could not say which half moved.
//!
//! Checking each file against its leaf is a third and finer answer and it is
//! `f-assembler`'s — `Refusal::Content` names the member. Nothing here
//! duplicates it: the frame does not instantiate topologies (RFC 0066), and a
//! second implementation of that check in the frame is the one thing the boot
//! module arrangement exists not to have.
//!
//! # Determinism
//!
//! No allocator, no clock, no collection to iterate: the offered modules are
//! visited in the order the caller yields them and the answer does not depend
//! on that order — a root selects at most one module, because two modules with
//! one root are two names for one generation and the first is as good as the
//! second. [`Chosen::at`] records which one so that a boot log can say *the
//! second module the loader placed*, which is the sentence somebody debugging a
//! menu entry needs.

use f_abi::boot::{MODULE_MAGIC, Module};
use f_hash::Sha256;

use crate::fold;
use crate::record::{Refusal, Tree};

/// One offered boot module, believed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Examined {
    /// The fold over the record tree inside it.
    /// Unit: bytes, exactly 32 — a SHA-256, the same one `cargo xtask
    /// generation` printed as `root`.
    pub root: [u8; 32],
    /// The SHA-256 of the whole module, as the loader delivered it: head,
    /// length array, record tree and every component file.
    /// Unit: bytes, exactly 32 — a SHA-256 over `bytes` of module.
    pub digest: [u8; 32],
    /// How long the module is.
    /// Unit: bytes.
    pub bytes: usize,
    /// How many members the record tree names, which is also how many component
    /// files the module carries — [`Module::read`] is what makes those one
    /// number rather than two.
    /// Unit: count of components.
    pub members: u16,
}

/// Why an offered boot module was not believed.
///
/// Only reachable for a blob whose first eight bytes are [`MODULE_MAGIC`]:
/// anything else is not a boot module and is skipped rather than refused, which
/// is `kernel/src/component.rs`'s rule one level up and for its reason — a
/// loader places `user/init`'s flat image and every component file on the same
/// list, and a frame that refused what it did not recognise would refuse every
/// boot this tree has ever run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The module's own head did not decode. Carries the packed `f_abi::error`
    /// [`Module::read`] refused with.
    Module(i32),
    /// The record tree inside it did not check.
    Tree(Refusal),
}

impl Rejected {
    /// A line for whoever is holding the menu entry that would not boot.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Module(_) => "the boot module's head is not one this build decodes",
            Self::Tree(_) => "the record tree in the boot module is not one this build believes",
        }
    }
}

/// Is this blob a boot module at all?
///
/// By magic and not by position or by name, for the reason `RFC 0030` gives one
/// level down: adding a generation to a menu entry must be a change to a
/// `module` line and not to the frame.
#[must_use]
pub fn is_module(bytes: &[u8]) -> bool {
    let Some(head) = bytes.get(..8) else {
        return false;
    };
    u64::from_le_bytes([head[0], head[1], head[2], head[3], head[4], head[5], head[6], head[7]])
        == MODULE_MAGIC
}

/// Believe one boot module, or say why not.
///
/// # Errors
///
/// A [`Rejected`], which names which of the two codecs refused it rather than
/// reporting that something was malformed. The caller is somebody holding a
/// root hash and a machine that will not boot, and *the module is wrong* is not
/// a sentence anybody can act on.
pub fn examine(bytes: &[u8]) -> Result<Examined, Rejected> {
    let module = Module::read(bytes).map_err(Rejected::Module)?;
    let tree = Tree::check(module.tree()).map_err(Rejected::Tree)?;
    let mut state = Sha256::new();
    state.update(bytes);
    Ok(Examined {
        root: fold::root(&tree),
        digest: state.finish(),
        bytes: bytes.len(),
        members: tree.members(),
    })
}

/// What a selection found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chosen {
    /// Where the selected module sat in the order the loader placed the blobs
    /// it was given — counting every blob, not only the boot modules, so that
    /// the number matches what a `menuentry`'s `module` lines say.
    /// Unit: index into the offered order, zero-based. `None` when nothing
    /// folded to the root asked for.
    pub at: Option<usize>,
    /// The selected module. `None` exactly when [`Chosen::at`] is.
    /// Unit: none — a summary, every field of which states its own.
    pub found: Option<Examined>,
    /// Blobs that were boot modules by magic.
    /// Unit: count of boot modules.
    pub offered: usize,
    /// Boot modules by magic that this build then refused.
    /// Unit: count of boot modules.
    pub rejected: usize,
    /// The first refusal, kept so that a machine offering one damaged module and
    /// nothing else can say what was wrong with it rather than only that it
    /// found nothing.
    /// Unit: none — a refusal.
    pub why: Option<Rejected>,
}

/// The one the boot was asked for, out of what the loader offered.
///
/// Blobs that are not boot modules are skipped in silence. Boot modules that do
/// not decode are counted and do not stop the search: a menu entry that lists
/// four generations and one of them is damaged should still boot the other
/// three, which is the whole reason a menu has more than one line.
#[must_use]
pub fn select<'a>(root: &[u8; 32], offered: impl Iterator<Item = &'a [u8]>) -> Chosen {
    let mut chosen = Chosen::default();
    for (at, bytes) in offered.enumerate() {
        if !is_module(bytes) {
            continue;
        }
        chosen.offered += 1;
        match examine(bytes) {
            Ok(found) if found.root == *root && chosen.at.is_none() => {
                chosen.at = Some(at);
                chosen.found = Some(found);
            }
            Ok(_) => {}
            Err(why) => {
                chosen.rejected += 1;
                if chosen.why.is_none() {
                    chosen.why = Some(why);
                }
            }
        }
    }
    chosen
}
