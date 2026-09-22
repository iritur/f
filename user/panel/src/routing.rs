// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where this component's needs landed, written by the frame and read by the
//! component, in one page neither of them has to guess the shape of.
//!
//! # The fifth copy of a layout constant, and the row that is new
//!
//! `user/virtio-blk/src/routing.rs` argues why a component is told and does not
//! compute, and the three crates after it record what writing the argument again
//! taught. [`AT`] equals every other component's and all of them equal
//! `kernel::process::BOARD`; `kernel/src/semantic.rs` asserts it at compile time
//! rather than saying it in a comment. RFC 0051 said at two drivers that this
//! constant belongs in `abi/` and RFC 0054 said it again at three; this is the
//! fifth and the argument has not improved.
//!
//! What is new here is [`at::TREE_HANDLE`], and it is the task. Every other
//! field on this page tells a component where something it will *use* is. This
//! one tells it what it **is allowed to do**, and to a thing it will never
//! address: the semantic tree lives in the frame's memory, in a page that was
//! never mapped here, and this component could not reach it if it wanted to. The
//! handle is not a pointer and it is not on the wire — `f_abi::semantic::Delta`
//! refuses a submission that names a capability, because *authority here is the
//! ring's* — so what it is for is quotation: the component writes it back at
//! [`reported::HANDLE`], and a boot that found the two disagreeing would be
//! looking at a component that held a different right from the one it was
//! granted.
//!
//! # The second half is the component's, and the frame only reads it
//!
//! Offsets from [`REPORT`] up are written by the *component* and read by the
//! frame after the run. RFC 0013's *read, never delivered*.

/// Which of this component's lives the frame asked for, in the low half of
/// `f_abi::door::Entry`.
pub mod life {
    /// Announce and end. What a spawn into a place asks for.
    pub const ANNOUNCE: u32 = 0;
    /// Adopt the ring, declare the interface, and end.
    ///
    /// **It ends by itself**, which is the one shape difference from every other
    /// component in this tree: a server runs until the frame's stop notice and
    /// an application runs until it has said what it came to say. So there is no
    /// control ring in this life and no `TOLD` outcome, and the frame learns the
    /// run is over the way it learns any process is — the join returns.
    pub const DECLARE: u32 = 1;
}

/// Where the frame maps this page in the component's address space.
///
/// Must equal `kernel::process::BOARD`. Unit: bytes, in the component's own
/// address space.
pub const AT: u64 = 0x0041_8000;

/// How many bytes the page is. One frame.
/// Unit: bytes.
pub const BYTES: u32 = 4096;

/// How many bytes the component's own state tree occupies.
///
/// One frame, which is what `kernel::component::publish_tree` writes into and
/// what `f_abi::state::Reader` is given to read back. RFC 0013 fixes the region
/// at one frame and `abi::manifest::STATE_NODES_MAX` is sized against it.
/// Unit: bytes.
pub const TREE_BYTES: u32 = 4096;

/// A word the frame writes first and the component checks before it believes
/// anything else here.
///
/// R04, at the one place a component reads a structure it did not build. It is
/// different from every other component's and has to be: each is mapped at
/// [`AT`] by the same shape, so a build that routed the wrong image into the
/// wrong supervisor would otherwise find a page whose magic matched and whose
/// fields meant something else.
pub const MAGIC: u64 = 0x7061_6E65_6C72_6F75;

/// How much heap this component's manifest declares.
///
/// One page, and **it is never allocated from**. This component has no
/// allocator: `crate`'s own comment says why an application that held a copy of
/// what it declared would be the arrangement section 11 is written against, and
/// a crate with no `alloc` cannot hold one by accident.
///
/// So why declare any. Because `kernel::process::prepare_server` describes a
/// heap region before the first instruction and a zero-length one is a region
/// `f_ring::heap::describe` has no prologue to write into — a shape this boot
/// would be the first to take, for a component that would not notice either way.
/// One page is the smallest thing that is a heap, and the number is written in
/// two places that are required to agree: here and in
/// `user/panel/manifest.toml`, checked by `kernel/src/semantic.rs` before a page
/// is spent.
/// Unit: bytes.
pub const HEAP_BYTES: u64 = 4096;

/// Byte offsets of the fields the frame writes, each a little-endian `u64`.
///
/// Slots rather than a `repr(C)` struct, because the two sides read and write
/// them through `f_ring::device::Window`, which is a bounds-checked volatile
/// accessor and not a reference — there is no struct to borrow. Eight bytes each
/// even where four would do, so that adding a field never moves one.
pub mod at {
    /// [`super::MAGIC`]. Unit: none.
    pub const MAGIC: u32 = 0;
    /// Where the ring this component declares across is.
    ///
    /// The component holds the **client's** end of it and the frame holds the
    /// server's, which is the opposite of every other component in this tree and
    /// is what an application is. Unit: bytes, in the component's address space.
    pub const DATA_AT: u32 = 8;
    /// How many bytes of it. Unit: bytes.
    pub const DATA_LEN: u32 = 16;
    /// Where this component's own state tree is.
    ///
    /// Told rather than assumed, for the reason `user/virtio-blk` gives: the
    /// tree is mapped by the shapes that publish one and by no other, so a
    /// component holding the address as a constant would fault at ring 3 on
    /// every boot that stood it up some other way. Zero means *this build
    /// published no tree for you*.
    /// Unit: bytes, in the component's address space.
    pub const TREE_AT: u32 = 24;
    /// The right this component holds over the semantic tree it is about to
    /// declare.
    ///
    /// `f_abi::cap::Handle`'s bits: a slot and a generation. **Not an address**
    /// — the tree is in the frame's own memory and is mapped nowhere in this
    /// address space — and not a field any entry carries, because
    /// `f_abi::semantic::Delta::envelope` refuses a submission naming a
    /// capability and says why: authority on a semantic channel is the ring's.
    ///
    /// So what is it for. It is what the component is *told it holds*, and it
    /// writes it back at [`super::reported::HANDLE`] so that the frame can check
    /// the two agree. That makes the handle a value this component demonstrably
    /// held, which is what `E3-B06c`'s second half needs: the frame keeps the
    /// same number, offers it after this component has ended, and is refused.
    /// Unit: none — a capability handle's bits.
    pub const TREE_HANDLE: u32 = 32;
    /// Which tree, as an address the system keeps rather than a right.
    ///
    /// `f_semantic::TreeId`'s index. Told for the same reason the handle is —
    /// so that a report can quote it — and it is a **separate word** because it
    /// is a separate thing: the address outlives the right, and a page that
    /// carried one number for both would be this task's finding folded back up.
    /// Unit: none — a registry slot index.
    pub const TREE_SLOT: u32 = 40;
    /// The ABI version the frame negotiated on this component's behalf.
    ///
    /// Read and refused rather than carried, for `f_compositor::routing`'s
    /// reason: a component built against a different ABI than the one its peer
    /// negotiated will misread an entry rather than fail to read one.
    /// Unit: none.
    pub const NEGOTIATED_VERSION: u32 = 48;
    /// The feature set beside it, whole. Refused when it is not empty, which is
    /// what this component's manifest declares.
    /// Unit: none — a bitmask of `f_abi::feature` constants.
    pub const NEGOTIATED_FEATURES: u32 = 56;
    /// How many turns this component will spend waiting for one completion
    /// before it gives up.
    ///
    /// RFC 0046 — a hang is a count. A count and not a duration, because RFC
    /// 0004 offers a component no clock. Unlike a server's, this bound is not a
    /// backstop: an application that submitted an entry and was never answered
    /// has nothing else to do, so reaching it is the run's real outcome and
    /// [`super::stopped::UNANSWERED`] is what it is called.
    /// Unit: turns.
    pub const IDLE_SPINS: u32 = 64;
}

/// The state nodes this component's manifest declares, by the id it declares
/// them under.
///
/// Here rather than in `component.rs` because both sides read them: the
/// component writes these ids and the frame reads them back out of the tree it
/// mounted. Ids are permanent and are never reused.
pub mod node {
    /// Entries this component put on the ring. Unit: entries.
    pub const SUBMITTED: u32 = 2;
    /// Completions it reaped. Unit: entries.
    pub const ANSWERED: u32 = 3;
    /// Completions carrying a refusal. Unit: entries.
    pub const REFUSED: u32 = 4;
    /// The handle it was granted over the tree it declared.
    /// Unit: none — a capability handle's bits.
    pub const HANDLE: u32 = 5;

    /// Every node this component writes a word into, in ascending id order.
    ///
    /// The component publishes exactly these and the frame requires exactly this
    /// many to carry a word, so a node added to the manifest and forgotten here
    /// is a boot that says three where the schema says four rather than a
    /// silence.
    /// Unit: none — node ids.
    pub const WRITTEN: [u32; 4] = [SUBMITTED, ANSWERED, REFUSED, HANDLE];
}

/// Where the component's own half of the page starts.
///
/// Half a page in, so that neither side can reach the other's fields by an
/// arithmetic slip of a few bytes. It is not protection — one page is one
/// mapping and the component may write all of it — it is distance.
/// Unit: bytes.
pub const REPORT: u32 = 2048;

/// Byte offsets of the fields the component writes.
pub mod reported {
    /// [`super::MAGIC`] again, written last, so that a frame reading a page the
    /// component never reached finds a zero rather than a plausible tally.
    pub const MAGIC: u32 = super::REPORT;
    /// Entries this component put on the ring. Unit: entries.
    pub const SUBMITTED: u32 = super::REPORT + 8;
    /// Completions it reaped, whatever they said. Unit: entries.
    pub const ANSWERED: u32 = super::REPORT + 16;
    /// Completions carrying a refusal, or answering an entry this component was
    /// not waiting for.
    ///
    /// Two things at once on purpose: a client that checked only the result
    /// would count the answer to entry three as the answer to entry four and
    /// never notice. Unit: entries.
    pub const REFUSED: u32 = super::REPORT + 24;
    /// The handle the frame told this component it holds, quoted back.
    ///
    /// The one field on this page that is not a count, and the one the boot's
    /// second half rests on: it is how the frame knows the number it offers
    /// after this component has ended is the number this component was holding
    /// while it ran. A zero here is a component that never read
    /// [`super::at::TREE_HANDLE`], which the frame refuses rather than reads as
    /// `Handle::NULL`.
    /// Unit: none — a capability handle's bits.
    pub const HANDLE: u32 = super::REPORT + 32;
    /// What stopped the component, as one of the [`stopped`](super::stopped)
    /// constants. Unit: none — an ordinal.
    pub const OUTCOME: u32 = super::REPORT + 40;
    /// How many of its declared state nodes this component wrote a word into.
    ///
    /// Published on the board *as well as* written into the tree, and the
    /// duplication is the point: the frame reads the tree itself and compares. A
    /// component that published nothing and claimed four here is caught by a
    /// reader that does not take its word for it.
    /// Unit: nodes.
    pub const PUBLISHED: u32 = super::REPORT + 48;
}

/// Why the component's run ended.
///
/// Written into [`reported::OUTCOME`] so that a boot can tell an application
/// that said what it came to say from one that fell over on something it read.
/// Both end; only one of them is the run the boot asked for.
pub mod stopped {
    /// Every entry of the script was submitted and answered without a refusal.
    pub const SAID: u64 = 1;
    /// The routing page did not carry [`super::MAGIC`], so nothing after it was
    /// believed.
    pub const NO_ROUTING: u64 = 2;
    /// An address in the routing page could not be stated as a window or a
    /// channel, or a negotiated value was not one this build speaks.
    pub const BAD_ROUTING: u64 = 3;
    /// The ring stopped validating under the component, which is a peer that has
    /// stopped speaking. RFC 0008.
    pub const NO_RING: u64 = 4;
    /// An entry was submitted and no completion arrived inside
    /// [`super::at::IDLE_SPINS`] turns.
    ///
    /// Its own outcome rather than [`NO_RING`], because they are different
    /// facts: a ring that stopped validating is a structure this component can
    /// see is wrong, and this is a peer that is simply not answering. An
    /// application told one when the other was true would report a broken
    /// channel to an operator who has a stopped service.
    pub const UNANSWERED: u64 = 5;
    /// A completion said the frame refused an entry of the script.
    ///
    /// Reported rather than retried. Every entry this component submits is one
    /// it built out of `f_semantic::Interface`, so a refusal is the two sides
    /// disagreeing about a vocabulary they are both reading out of the same
    /// crate — which is not a thing a retry fixes.
    pub const TURNED_DOWN: u64 = 6;
    /// This build's own vocabulary did not admit this build's own roles.
    ///
    /// `interface/src/node.rs` and `semantic/src/vocabulary.rs` disagreeing,
    /// which is a build failure wearing a run-time refusal. It has a word
    /// anyway, because a component has no unwinder: the alternative to
    /// answering it is an `unwrap` that stops the core with nothing written
    /// anywhere, and a boot reading a dead core cannot tell that from a fault.
    pub const NO_SCRIPT: u64 = 8;
    /// The frame told this component it holds no handle over any tree.
    ///
    /// Refused before a single entry is submitted, and it is the one refusal
    /// this component makes about its own authority: an application that
    /// declared an interface it was granted no right to declare would have every
    /// entry turned down one at a time, and the boot would be reading six
    /// refusals where the cause was one absent word.
    pub const NO_HANDLE: u64 = 7;
}
