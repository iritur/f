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
//! an instruction. That decision is `crate::policy`'s and has been since RFC
//! 0076; what the frame copies onto this page since `E3-B05e` includes two words
//! an occupant published about its own progress, and the frame copying them
//! without reading them is the whole of why they may be here.
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
    /// How many rows of [`ROW`] are real. Unit: places.
    pub const PLACES: u32 = 32;
    /// The logical tick this consultation is stamped with.
    ///
    /// **Not a clock**, and the distinction is the whole of RFC 0004 here. It is
    /// a count the frame advances by the backoff a verdict returned, which is
    /// what a supervisor's own clock would be if it had one — so two boots of
    /// one commit agree, and a seeded scenario can drive a restart storm without
    /// a wall-clock accident. `kernel/src/component.rs` holds it and never
    /// interprets it.
    /// Unit: timer ticks, at the frame's own rate.
    pub const NOW: u32 = 40;

    // --- one row per place, written by the frame ------------------------------

    /// Where the generation this machine is was mapped, in this component's own
    /// address space, or zero for a boot that selected none.
    ///
    /// **In the gap between [`NOW`] and [`ROW`] rather than after the rows**, so
    /// that nothing existing moves: a field added inside the row block would
    /// shift every row after the first, and a row's offset is arithmetic both
    /// sides do. RFC 0094.
    /// Unit: bytes.
    pub const MODULE_AT: u32 = 48;

    /// How long it is. Unit: bytes.
    pub const MODULE_LEN: u32 = 56;

    /// Where the rows begin. Unit: bytes.
    pub const ROW: u32 = 64;
    /// How far apart two rows are. Unit: bytes.
    ///
    /// Wider than the fields need, so that a field added to a row never moves
    /// one — which on a page two crates read is the difference between a new
    /// field and a silent reinterpretation of every row after the first.
    pub const ROW_STRIDE: u32 = 40;
    /// The place's manifest, by content hash — what
    /// `f_abi::control::op::SPAWN` carries in `ext[0]`. Offset within a row.
    ///
    /// A content hash and not an index, because an index is a statement about
    /// the order the loader placed modules in and a hash is a statement about
    /// what the module *is*. The boot that swapped places 2 and 3 on hardware
    /// (`TODO.md` E0-P18) is why that distinction is load-bearing rather than
    /// fastidious.
    /// Unit: none — an `f_abi::ContentId`.
    pub const ROW_MANIFEST: u32 = 0;
    /// The place's endpoint, as a handle **in this component's own table**.
    ///
    /// This is what makes a supervisor tellable at all (RFC 0076): a place's
    /// `PEER_GONE` pends in the capability slot of the endpoint that names it,
    /// so a supervisor that held no endpoint would be a supervisor the frame has
    /// no slot to leave a death in. It is also what `op::STOP` is checked
    /// against, so the same handle answers both halves of a lifecycle.
    ///
    /// A notice arrives with this value in `Cqe::user_data`, which is how a row
    /// is found from a death.
    /// Unit: none — a capability handle.
    pub const ROW_ENDPOINT: u32 = 8;
    /// The restarts already spent inside the current window, as this supervisor
    /// last left it. Unit: restarts.
    pub const ROW_USED: u32 = 16;
    /// When that window opened, in [`NOW`]'s ticks. Unit: timer ticks.
    pub const ROW_OPENED: u32 = 24;
    /// Whether the place has an occupant **now**: the occupant's epoch plus
    /// one, or zero for an empty place.
    ///
    /// **A fact and not an instruction**, and the difference is the one this
    /// page's module comment is built on. Until `E3-B05e` a supervisor inferred
    /// *the frame built this place and never filled it* from a tally nobody had
    /// touched, which was sound while every row it was shown was empty. A row
    /// with a live occupant and an untouched tally — the compositor's, on the
    /// boot that carries its liveness — reads identically under that rule, and a
    /// supervisor that followed it would submit a spawn into an occupied place.
    /// So the frame says what it knows, and the component still decides.
    ///
    /// Plus one for [`f_abi::swap`]'s reason: an epoch counts from zero and the
    /// first occupant must not read as no occupant at all.
    /// Unit: none — an epoch ordinal, offset by one.
    pub const ROW_OCCUPANT: u32 = 32;

    /// The root the module folds to, thirty-two bytes.
    ///
    /// **Written by the frame and never computed by the reader**, which is the
    /// whole reason it is here. `f_assembler::Assembly::instantiate` refolds the
    /// module and compares it against this; a component that folded the module
    /// to get the root it then compared against would be checking the bytes
    /// against themselves, and the check would pass for any module at all.
    ///
    /// In the gap that begins where the four rows end (64 + 4 × 40 = 224) and
    /// runs to [`SUBMITTED`], so nothing existing moves.
    /// Unit: none — a SHA-256.
    pub const ROOT: u32 = 224;

    /// How many bytes of it. A length beside an address, because a reader that
    /// took the length from a constant would be a reader that stops agreeing the
    /// day the hash does.
    /// Unit: bytes.
    pub const ROOT_BYTES: usize = 32;

    // --- one liveness row per place, written by the frame ---------------------
    //
    // **`E3-B05e`'s delivery, and every word of it is copied.** Two numbers a
    // synchronising occupant published about itself, and this supervisor's own
    // memory of the second, stored by the frame between runs exactly as
    // [`ROW_USED`] and [`ROW_OPENED`] are. The frame never compares them, adds to
    // them or branches on them — RFC 0123 names the day it does as the day RFC
    // 0008's objection lands — and `cargo xtask lint-datapath` refuses a frame
    // that names this crate's `policy::` at all.
    //
    // A block of its own rather than two more fields in a row, because a row is
    // [`ROW_STRIDE`] wide and five fields fill it; widening the stride would move
    // [`ROOT`], which is arithmetic both sides do. It starts where the root ends.

    /// Where the liveness rows begin. Unit: bytes.
    pub const LIVE: u32 = 256;
    /// How far apart two of them are. Unit: bytes.
    pub const LIVE_STRIDE: u32 = 24;
    /// Waits the occupant's last closed frame entered and did not get out of,
    /// as it published them. Offset within a liveness row.
    /// Unit: waits.
    pub const LIVE_WAITS: u32 = 0;
    /// Frames the occupant has abandoned, as it published them. Offset within a
    /// liveness row. Unit: frames.
    pub const LIVE_ABANDONED: u32 = 8;
    /// This supervisor's memory of [`LIVE_ABANDONED`] as it last read it, handed
    /// back so that one stuck frame is one fate and not one per consultation.
    /// Offset within a liveness row. Unit: frames.
    pub const LIVE_SEEN: u32 = 16;

    // --- what the component writes, and the frame reads afterwards ------------

    /// How many spawns this supervisor put on its control ring. Unit: entries.
    pub const SUBMITTED: u32 = 384;
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
    pub const REFUSED: u32 = 392;
    /// How many deaths this supervisor was told about on its ring.
    /// Unit: notices.
    pub const TOLD: u32 = 400;

    /// Where the component's own per-place rows begin. Unit: bytes.
    pub const SAID: u32 = 448;
    /// How far apart two of those are. Unit: bytes.
    pub const SAID_STRIDE: u32 = 24;

    /// What the component made of the generation it was shown, written back.
    ///
    /// `f_assembler::render::digest` over the assembly — the topology's own
    /// rendering, not the module it came from. That distinction is
    /// `user/assembler/src/render.rs`'s central one: the module is the *input*,
    /// and a digest over the input would be a hash comparison wearing an
    /// assembler's clothes.
    ///
    /// **This is what turns `E2-B05`'s exit from a property of an assembly into
    /// a property of a boot.** *Boot is a pure function of one hash* is a claim
    /// about what a machine did, and until something on the machine published
    /// this number it was a claim about a host-side test.
    /// Unit: none — a SHA-256.
    pub const DIGEST: u32 = 544;

    /// How many members the assembler started, failed, found no device for, and
    /// left unstarted because something they depend on did not start.
    ///
    /// Four counts in the order [`f_assembler::start::Report`] declares them, so
    /// a reader comparing the two does not have to hold a mapping in their head.
    /// Unit: members.
    pub const STARTED: u32 = 576;
    /// See [`STARTED`]. Unit: members.
    pub const FAILED: u32 = 584;
    /// See [`STARTED`]. Unit: members.
    pub const ABSENT: u32 = 592;
    /// See [`STARTED`]. Unit: members.
    pub const UNSTARTED: u32 = 600;

    /// Members the supervisor did not try, because the frame had already filled
    /// their places.
    ///
    /// **Not a failure, and counted apart so that it cannot be read as one.**
    /// The frame holds exactly one place open today; every other member of the
    /// topology is a place it filled itself before the supervisor ran. A `Start`
    /// implementation that answered `Err` for those would mark each one's whole
    /// subtree `Unstarted` and make the rendered topology claim a boot failed
    /// that did not. RFC 0094 states the distinction as the load-bearing one.
    /// Unit: members.
    pub const SKIPPED: u32 = 608;

    /// Why the assembler refused, if it did, as `f_assembler::Refusal`'s
    /// discriminant plus one — zero being *it did not*.
    ///
    /// A refusal does not fail the boot. The supervisor falls back to the row
    /// loop it ran before RFC 0094, because a malformed module must not turn a
    /// machine that boots into one that does not: `docs/booting-on-hardware.md`
    /// makes every component file optional and this is the same argument one
    /// level up.
    /// Unit: none — an ordinal.
    pub const REFUSAL: u32 = 616;
    /// What this supervisor decided about the row, as
    /// `crate::policy::Verdict::to_wire`. Offset within a said-row.
    ///
    /// **The frame performs this and does not compute it.** A retirement is the
    /// one verdict with no opcode behind it — there is nothing on the control
    /// ring that says *end this place* as opposed to *end its occupant* — so it
    /// travels here instead, and the frame does the revoking. That split is RFC
    /// 0008's exactly: the decision is the supervisor's, the mechanism is the
    /// frame's.
    /// Unit: none — an ordinal.
    pub const SAID_VERDICT: u32 = 0;
    /// The tally as this supervisor left it, to be stored and handed back on the
    /// next consultation. Unit: restarts.
    pub const SAID_USED: u32 = 8;
    /// And when its window opened. Unit: timer ticks.
    pub const SAID_OPENED: u32 = 16;

    /// Where the component's account of **what it read** begins, one row per
    /// place. Unit: bytes.
    ///
    /// # Why a supervisor writes back what it was told
    ///
    /// Because `E3-B05e` found a policy that had been deciding from a word it
    /// was never given. The cause of a death has been specified as the `ext` of
    /// a peer-gone notice since RFC 0008, the supervisor's drain read it from
    /// there, and the frame posted zero: so every restart this boot had ever
    /// shown came from the branch for *a place the frame built and never
    /// filled*, `policy::decide` was never called on a machine, and the budget
    /// line said `restart 0 of 3` in every log since. Nothing noticed, because
    /// nothing on either side of the boundary could see what the other had.
    ///
    /// So the component says what it heard, and the frame — which knows what it
    /// posted, because the cause is its own word — requires the two to be one.
    /// The liveness words it heard are here for the same reason and are checked
    /// by `cargo xtask compositor` rather than by the frame, because comparing
    /// those is the one thing RFC 0123 forbids the frame.
    ///
    /// After [`REFUSAL`], in space the component half has not used.
    pub const HEARD: u32 = 624;
    /// How far apart two of those are. Unit: bytes.
    pub const HEARD_STRIDE: u32 = 32;
    /// The packed cause the peer-gone notice for this row carried, or zero for
    /// a row nothing died in this run. Offset within a heard-row.
    /// Unit: none — an `f_abi::control::cause` word.
    pub const HEARD_CAUSE: u32 = 0;
    /// [`LIVE_WAITS`] as this component read it. Unit: waits.
    pub const HEARD_WAITS: u32 = 8;
    /// [`LIVE_ABANDONED`] as this component read it. Unit: frames.
    pub const HEARD_ABANDONED: u32 = 16;
    /// What this supervisor will remember as [`LIVE_SEEN`] next run.
    ///
    /// Its own word and not derived by the frame from [`HEARD_ABANDONED`], even
    /// though today they are the same number: one is evidence and the other is
    /// policy memory, and a frame that computed the second from the first would
    /// be deciding what a supervisor remembers.
    /// Unit: frames.
    pub const HEARD_SEEN: u32 = 24;

    // --- one policy row per place, written by the frame -----------------------
    //
    // **The reversal `component::declared` named, paid.** A supervisor holds a
    // content hash, which names a manifest and does not reach it, so the four
    // numbers `policy::decide` reads were a function returning `user/store`'s —
    // *right today and right by coincidence*, with a paragraph saying the day it
    // stops being right is the day two supervised places declare different
    // policies. `E3-B05e` is that day: the compositor declares eight restarts in
    // sixty thousand ticks where `user/store` declares three in three thousand,
    // and a timeout decided against the store's budget would be a restart line
    // naming one manifest's number over another's decision.
    //
    // Above everything the component writes rather than below [`SUBMITTED`],
    // because the gap there after the liveness rows is thirty-two bytes and one
    // of these rows is forty. The gap between [`HEARD`]'s end and this is room.

    /// Where the policy rows begin. Unit: bytes.
    pub const POLICY: u32 = 1024;
    /// How far apart two of them are. Unit: bytes.
    pub const POLICY_STRIDE: u32 = 40;
    /// `f_abi::manifest::Record::restart`. Unit: none — a `restart` ordinal.
    pub const POLICY_RESTART: u32 = 0;
    /// `Record::max_restarts`. Unit: restarts.
    pub const POLICY_MAX_RESTARTS: u32 = 8;
    /// `Record::budget_window_ticks`. Unit: timer ticks.
    pub const POLICY_WINDOW: u32 = 16;
    /// `Record::backoff_first_ticks`. Unit: timer ticks.
    pub const POLICY_BACKOFF_FIRST: u32 = 24;
    /// `Record::backoff_max_ticks`. Unit: timer ticks.
    pub const POLICY_BACKOFF_MAX: u32 = 32;
}

// The layout, held by the compiler rather than by the paragraphs above. Every
// block added to this page since RFC 0094 has been placed *in a gap*, and a gap
// computed by hand is the arithmetic that goes wrong silently: two blocks that
// overlap write each other's words and both sides read a plausible number.
const _: () = assert!(at::ROW_OCCUPANT + 8 <= at::ROW_STRIDE);
const _: () = assert!(at::ROW + PLACES_MAX as u32 * at::ROW_STRIDE <= at::ROOT);
const _: () = assert!(at::ROOT + at::ROOT_BYTES as u32 <= at::LIVE);
const _: () = assert!(at::LIVE_SEEN + 8 <= at::LIVE_STRIDE);
const _: () = assert!(at::LIVE + PLACES_MAX as u32 * at::LIVE_STRIDE <= at::SUBMITTED);
const _: () = assert!(at::SAID + PLACES_MAX as u32 * at::SAID_STRIDE <= at::DIGEST);
const _: () = assert!(at::REFUSAL + 8 <= at::HEARD);
const _: () = assert!(at::HEARD_SEEN + 8 <= at::HEARD_STRIDE);
const _: () = assert!(at::HEARD + PLACES_MAX as u32 * at::HEARD_STRIDE <= at::POLICY);
const _: () = assert!(at::POLICY_BACKOFF_MAX + 8 <= at::POLICY_STRIDE);
const _: () = assert!(at::POLICY + PLACES_MAX as u32 * at::POLICY_STRIDE <= BYTES);

/// The four numbers `crate::policy::decide` reads, and the restart policy they
/// are read under, as the frame copied them out of the place's own manifest.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Policy {
    /// `f_abi::manifest::restart`'s ordinal.
    pub restart: u8,
    /// Unit: restarts.
    pub max_restarts: u32,
    /// Unit: timer ticks.
    pub window_ticks: u32,
    /// Unit: timer ticks.
    pub backoff_first_ticks: u32,
    /// Unit: timer ticks.
    pub backoff_max_ticks: u32,
}

impl Policy {
    /// As the record `decide` takes.
    ///
    /// Built from `Record::EMPTY` so that a field the format gains arrives as
    /// the zero it arrives as everywhere else, rather than as a value this
    /// supervisor invented.
    #[must_use]
    pub const fn record(self) -> f_abi::manifest::Record {
        f_abi::manifest::Record {
            restart: self.restart,
            max_restarts: self.max_restarts,
            budget_window_ticks: self.window_ticks,
            backoff_first_ticks: self.backoff_first_ticks,
            backoff_max_ticks: self.backoff_max_ticks,
            ..f_abi::manifest::Record::EMPTY
        }
    }
}

/// What this supervisor made of one row, to be written back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Said {
    /// The verdict, as [`crate::policy::Verdict::to_wire`] will spell it.
    pub verdict: u64,
    /// The tally, to be stored on the place.
    pub budget: crate::policy::Budget,
    /// The cause the notice for this row carried, packed, or zero.
    pub cause: u64,
    /// The liveness it read.
    pub heard: crate::policy::Liveness,
    /// What it will remember as `seen`.
    pub seen: u64,
}

/// One place this supervisor may act on, as the frame described it.
#[derive(Clone, Copy, Default)]
pub struct Row {
    /// The manifest to put in it, by content hash.
    pub manifest: u64,
    /// Its endpoint, in this component's own table. A death arrives carrying
    /// this in `Cqe::user_data`.
    pub endpoint: u32,
    /// The tally the frame is holding for this place, handed back so that a
    /// window spans runs rather than restarting with the supervisor.
    pub budget: crate::policy::Budget,
    /// [`at::ROW_OCCUPANT`]: the occupant's epoch plus one, or zero for an
    /// empty place.
    pub occupant: u64,
    /// What the occupant published about its own progress, copied by the frame.
    /// Zeroes for a place whose occupant published nothing of the kind.
    pub liveness: crate::policy::Liveness,
    /// This supervisor's memory of the abandoned count, as it last left it.
    /// Unit: frames.
    pub seen: u64,
    /// The place's own restart policy, out of its own manifest.
    pub policy: Policy,
}

/// What the frame wrote, read once and believed thereafter.
///
/// Copied out rather than re-read, which is `f_ring::Mapping`'s discipline and
/// the reason that file gives: validating a page in place and then acting on a
/// later read of it is a check that bounds nothing. Nothing else writes this
/// page while this component runs — the frame fills it before the first
/// instruction and reads the far half after the last one — but the copy is
/// cheap and does not depend on that staying true.
#[derive(Clone, Copy)]
pub struct Board {
    /// Where the control ring is. Unit: bytes, in this address space.
    pub control_at: u64,
    /// How many bytes of it. Unit: bytes.
    pub control_len: u32,
    /// The account this supervisor may spend.
    pub account: u32,
    /// The logical tick this consultation is stamped with.
    pub now: u64,
    /// Where the generation this machine is was mapped, or zero for a boot that
    /// selected none. Unit: bytes, in this address space.
    pub module_at: u64,
    /// How long it is — the module's own length, not the extent it was mapped
    /// over. Reading the second would read past the module into whatever the
    /// loader put after it. Unit: bytes.
    pub module_len: u32,
    /// What it folds to.
    ///
    /// **The frame's answer and never this component's**, which is the whole
    /// reason it crosses the board: `f_assembler::Assembly::instantiate` refolds
    /// the module and compares against this, and a reader that folded the module
    /// to get the root it then compared against would be checking the bytes
    /// against themselves.
    /// Unit: none — a SHA-256.
    pub root: [u8; at::ROOT_BYTES],
    /// The places it may act on, in the frame's order.
    pub rows: [Row; PLACES_MAX],
    /// How many of `rows` are real. Unit: places.
    pub places: usize,
}

impl Board {
    /// Read the page the frame filled in.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/MALFORMED_HEADER` for a page whose magic is not [`MAGIC`] —
    /// which is the frame not having written it, and is refused before any other
    /// field is looked at.
    ///
    /// `RESOURCE/QUOTA_EXHAUSTED` for a page naming more places than
    /// [`PLACES_MAX`], so that a build which outgrew this page says so instead
    /// of supervising a prefix of itself.
    ///
    /// Whatever `f_ring::device::Window` refuses, for a page that is not there.
    pub fn read() -> Result<Self, i32> {
        let quota =
            f_abi::error::pack(f_abi::error::RESOURCE, f_abi::error::resource::QUOTA_EXHAUSTED);
        let page = f_ring::device::Window::at(AT, BYTES)?;
        if page.read64(at::MAGIC)? != MAGIC {
            return Err(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::MALFORMED_HEADER,
            ));
        }
        let places = usize::try_from(page.read64(at::PLACES)?).unwrap_or(usize::MAX);
        if places > PLACES_MAX {
            return Err(quota);
        }
        let mut rows = [Row::default(); PLACES_MAX];
        for (index, row) in rows.iter_mut().enumerate().take(places) {
            let base =
                at::ROW + u32::try_from(index).map_err(|_| quota)?.saturating_mul(at::ROW_STRIDE);
            row.manifest = page.read64(base + at::ROW_MANIFEST)?;
            row.endpoint = u32::try_from(page.read64(base + at::ROW_ENDPOINT)?).unwrap_or(0);
            row.budget = crate::policy::Budget {
                used: u32::try_from(page.read64(base + at::ROW_USED)?).unwrap_or(0),
                opened: page.read64(base + at::ROW_OPENED)?,
            };
            row.occupant = page.read64(base + at::ROW_OCCUPANT)?;
            let index = u32::try_from(index).map_err(|_| quota)?;
            let live = at::LIVE + index.saturating_mul(at::LIVE_STRIDE);
            row.liveness = crate::policy::Liveness {
                outstanding_waits: page.read64(live + at::LIVE_WAITS)?,
                abandoned_frames: page.read64(live + at::LIVE_ABANDONED)?,
            };
            row.seen = page.read64(live + at::LIVE_SEEN)?;
            // A word that does not fit the field it names is refused rather than
            // truncated, which is R04 at the one read here whose value decides a
            // restart: a policy of `max_restarts = 2^32 + 3` read as three would
            // be this supervisor enforcing a budget nobody declared.
            let policy = at::POLICY + index.saturating_mul(at::POLICY_STRIDE);
            let narrow = |offset: u32| -> Result<u32, i32> {
                u32::try_from(page.read64(policy + offset)?).map_err(|_| quota)
            };
            row.policy = Policy {
                restart: u8::try_from(page.read64(policy + at::POLICY_RESTART)?)
                    .map_err(|_| quota)?,
                max_restarts: narrow(at::POLICY_MAX_RESTARTS)?,
                window_ticks: narrow(at::POLICY_WINDOW)?,
                backoff_first_ticks: narrow(at::POLICY_BACKOFF_FIRST)?,
                backoff_max_ticks: narrow(at::POLICY_BACKOFF_MAX)?,
            };
        }
        let mut root = [0u8; at::ROOT_BYTES];
        for (index, chunk) in root.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let word = page.read64(at::ROOT + u32::try_from(index).map_err(|_| quota)? * 8)?;
            *chunk = word.to_le_bytes();
        }
        Ok(Self {
            module_at: page.read64(at::MODULE_AT)?,
            module_len: u32::try_from(page.read64(at::MODULE_LEN)?).unwrap_or(0),
            root,
            control_at: page.read64(at::CONTROL_AT)?,
            control_len: u32::try_from(page.read64(at::CONTROL_LEN)?).unwrap_or(0),
            account: u32::try_from(page.read64(at::ACCOUNT)?).unwrap_or(0),
            now: page.read64(at::NOW)?,
            rows,
            places,
        })
    }

    /// Write back what this run decided, and what it did.
    ///
    /// Ignores its own refusals, and that is deliberate rather than lazy: this
    /// is the last thing the component does before [`f_abi::door::EXIT`], and a
    /// supervisor that died reporting would be a supervisor whose work the frame
    /// then could not see. The frame's own place table is what the boot checks;
    /// this is the component's account of itself beside it.
    pub fn report(totals: (u64, u64, u64), said: &[Said]) {
        let Ok(page) = f_ring::device::Window::at(AT, BYTES) else { return };
        let (submitted, refused, told) = totals;
        let _ = page.write64(at::SUBMITTED, submitted);
        let _ = page.write64(at::REFUSED, refused);
        let _ = page.write64(at::TOLD, told);
        for (index, row) in said.iter().enumerate().take(PLACES_MAX) {
            let Ok(index) = u32::try_from(index) else { return };
            let base = at::SAID + index.saturating_mul(at::SAID_STRIDE);
            let _ = page.write64(base + at::SAID_VERDICT, row.verdict);
            let _ = page.write64(base + at::SAID_USED, u64::from(row.budget.used));
            let _ = page.write64(base + at::SAID_OPENED, row.budget.opened);
            let heard = at::HEARD + index.saturating_mul(at::HEARD_STRIDE);
            let _ = page.write64(heard + at::HEARD_CAUSE, row.cause);
            let _ = page.write64(heard + at::HEARD_WAITS, row.heard.outstanding_waits);
            let _ = page.write64(heard + at::HEARD_ABANDONED, row.heard.abandoned_frames);
            let _ = page.write64(heard + at::HEARD_SEEN, row.seen);
        }
    }

    /// Write back what the assembler made of the generation.
    ///
    /// **This is what turns `E2-B05`'s exit from a property of an assembly into
    /// a property of a boot.** *Boot is a pure function of one hash* is a claim
    /// about what a machine did, and until a machine published this number it
    /// was a claim about a host-side test. The frame reads it off this page after
    /// the core comes back and puts it in the boot log.
    #[cfg(all(target_arch = "x86_64", feature = "image"))]
    pub fn assembled(assembled: &crate::assemble::Assembled) {
        let Ok(page) = f_ring::device::Window::at(AT, BYTES) else { return };
        for (index, chunk) in assembled.digest.as_chunks::<8>().0.iter().enumerate() {
            let Ok(index) = u32::try_from(index) else { return };
            let _ = page.write64(at::DIGEST + index * 8, u64::from_le_bytes(*chunk));
        }
        let _ = page.write64(at::STARTED, assembled.report.started as u64);
        let _ = page.write64(at::FAILED, assembled.report.failed as u64);
        let _ = page.write64(at::ABSENT, assembled.report.absent as u64);
        let _ = page.write64(at::UNSTARTED, assembled.report.unstarted as u64);
        let _ = page.write64(at::SKIPPED, assembled.skipped);
        let _ = page.write64(at::REFUSAL, assembled.refusal);
    }
}
