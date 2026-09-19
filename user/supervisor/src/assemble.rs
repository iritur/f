// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Instantiate the generation this machine was asked to be, and start what the
//! topology says to start.
//!
//! # Why this is here and not in the frame
//!
//! RFC 0066 landed `user/assembler` above the frame with no caller, because the
//! frame cannot link it: the crate allocates — one `BTreeMap` and three `Vec`s
//! — and there is no `#[global_allocator]` under `kernel/`, nor may there be.
//! That RFC named two conditions that would reverse it, and **both arrived**:
//! `E1-B05`'s restart policy left the frame, and `f_ring::heap` landed. It also
//! predicted the shape this module has, to the letter — *the supervisor is a
//! component, `start::Start` is implemented by it as a `SPAWN` on a control
//! ring, and what changes is one impl, not this crate*.
//!
//! One impl is what this is. RFC 0094.
//!
//! # What the frame does and what this does
//!
//! The frame **selects**: it reads `f.root=` off the command line, folds every
//! module the loader offered, and refuses to boot a machine that is not the
//! generation it was told to be. That much is `kernel/src/generation.rs` and it
//! is unchanged.
//!
//! This **instantiates**: it refolds the module with the same `f_generation::
//! fold` that produced the root, recomputes every member's content address over
//! the bytes that arrived, resolves the routes the tree declares, and walks the
//! start order. The frame does none of that and holds none of the results.
//!
//! The two halves meet at a number. The frame writes the root onto this
//! component's board and this component hands it to
//! [`f_assembler::topology::Assembly::instantiate`], which compares. A reader
//! wondering why the root crosses a page rather than being recomputed here has
//! the answer in that sentence: a component that folded the module to obtain the
//! root it then compared against would be checking the bytes against themselves,
//! and the check would pass for any module at all.
//!
//! # The seam worth knowing about
//!
//! **Two content addresses name the same bytes and they are not the same
//! number.** `f_assembler::topology::Instance::content` is a SHA-256, because
//! the generation tree folds under SHA-256 and the assembler's whole claim is
//! arithmetic over that fold. `f_abi::control::op::SPAWN` carries an
//! `f_abi::manifest::ContentId`, which is FNV-1a, because that is what a place
//! holds and what the frame's spawn server compares against.
//!
//! So [`Spawning::start`] recomputes `ContentId::of(module.file(index))` rather
//! than forwarding the address the assembler already has. The two agree by
//! construction — the packer writes the identical `.fc` bytes the loose file
//! carries — and **nothing checks that they do**. RFC 0094's reversal section is
//! where that is written down.

use alloc::vec::Vec;

use f_abi::manifest::{ContentId, Record};
use f_assembler::bind::{Address, Bus};
use f_assembler::start::{Report, Start};
use f_assembler::topology::{Assembly, Refusal};

/// What a run of the assembler produced, for the board.
#[derive(Clone, Default)]
pub struct Assembled {
    /// `f_assembler::render::digest` over the assembly.
    ///
    /// **The topology's rendering and not the module's bytes.** The module is
    /// the *input*, and a digest over an input would be a hash comparison
    /// wearing an assembler's clothes — `user/assembler/src/render.rs` argues
    /// that at length and this is the one place the argument is spent.
    pub digest: [u8; 32],
    /// What [`f_assembler::start::start`] answered.
    pub report: Report,
    /// Members this component did not try, because the frame had already filled
    /// their places. Unit: members.
    pub skipped: u64,
    /// The refusal, as a discriminant plus one, or zero for a run that was not
    /// refused. Unit: none — an ordinal.
    pub refusal: u64,
    /// Spawns this run actually put on the control ring. Unit: entries.
    ///
    /// **Not `report.started`**, which is the count of members `Start::start`
    /// answered `Ok` for — and a skipped member answers `Ok`, because a member
    /// the frame already filled is not a failure. Reporting that number as
    /// *submitted* made the boot log say three spawns went on a ring that had
    /// one, which is the kind of number that is only wrong once somebody checks.
    pub submitted: u64,
    /// Which board rows this run put a spawn on the ring for.
    ///
    /// **The load-bearing output beside the digest**, and it exists because of a
    /// boot that hung on the difference. The loop after this one refills a place
    /// whose tally is untouched, reading that as *the frame built this place and
    /// deliberately never filled it*; a row the assembler has just submitted for
    /// also has an untouched tally, and refilling it would be two spawns for one
    /// place — the second of which the frame refuses, which is this component
    /// arguing with itself in the boot log.
    ///
    /// Answering *the assembler ran* is not enough to tell those apart, because a
    /// run that skipped every member leaves every row exactly as it found it.
    /// This says which rows it actually took.
    pub filled: [bool; crate::routing::PLACES_MAX],
}

/// The `Start` the assembler is given: a spawn on the control ring.
///
/// Holds the module because the id it submits is recomputed from it — see the
/// seam in this file's head — and the board's rows because a spawn names a place
/// the frame is holding open, and the board is the only thing that maps a
/// manifest to one.
pub struct Spawning<'a> {
    module: f_abi::boot::Module<'a>,
    control: &'a f_ring::adopt::Client,
    account: u32,
    /// `(manifest, filled)` per row, in the frame's order.
    pub rows: [(u64, bool); crate::routing::PLACES_MAX],
    places: usize,
    /// Spawns put on the ring. Unit: entries.
    pub submitted: u64,
    /// Spawns the ring had no room for. Unit: entries.
    pub refused: u64,
    /// Members with no row to start them into. Unit: members.
    pub skipped: u64,
}

impl<'a> Spawning<'a> {
    /// Bind one to a module and a board.
    pub fn new(
        module: f_abi::boot::Module<'a>,
        control: &'a f_ring::adopt::Client,
        board: &crate::routing::Board,
    ) -> Self {
        let mut rows = [(0u64, false); crate::routing::PLACES_MAX];
        for (slot, row) in rows.iter_mut().zip(board.rows.iter()).take(board.places) {
            *slot = (row.manifest, false);
        }
        Self {
            module,
            control,
            account: board.account,
            rows,
            places: board.places,
            submitted: 0,
            refused: 0,
            skipped: 0,
        }
    }
}

impl Start for Spawning<'_> {
    /// Start one member, by asking the frame to.
    ///
    /// # Why a member with no row answers `Ok`
    ///
    /// Because it is not a failure, and saying it was would be a lie with
    /// consequences. `f_assembler::start::start` marks the whole **subtree** of a
    /// failed member `Unstarted`, which is `E2-B05`'s second exit clause working
    /// as designed. The frame holds exactly one place open for this component
    /// today and fills the other five itself before this runs, so answering
    /// `Err` for those five would render a topology claiming a boot failed that
    /// did not.
    ///
    /// Counted apart, on the board, under its own name. A skipped member and a
    /// failed one are different news and the log says which.
    ///
    /// # Errors
    ///
    /// The packed refusal the ring gave, for a submission that did not happen.
    /// A full ring is the one refusal this component can see for itself and it is
    /// a real one: nothing drains this ring while this component holds the core.
    fn start(&mut self, index: u16, _record: &Record, _at: Option<Address>) -> Result<(), i32> {
        let quota =
            f_abi::error::pack(f_abi::error::RESOURCE, f_abi::error::resource::QUOTA_EXHAUSTED);
        let Some(file) = self.module.file(index as usize) else {
            return Err(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::BAD_ADDRESS,
            ));
        };
        // FNV-1a over the bytes that arrived, and **not** the assembler's
        // SHA-256 for the same member. This file's head says why.
        let id = ContentId::of(file).bits();

        let found = self
            .rows
            .iter_mut()
            .take(self.places)
            .find(|(manifest, filled)| *manifest == id && !*filled);
        let Some((_, filled)) = found else {
            self.skipped += 1;
            return Ok(());
        };
        *filled = true;

        let entry = f_abi::Sqe {
            opcode: f_abi::control::op::SPAWN,
            cap: self.account,
            // The row, plus one, because zero is the `user_data` an entry nobody
            // set carries and an answer matched against it would match the
            // frame's own notices. The same convention `supervise` uses.
            user_data: u64::from(index) + 1,
            ext: [id, 0],
            ..f_abi::Sqe::ZERO
        };
        match self.control.submit(entry) {
            Ok(_) => {
                self.submitted += 1;
                Ok(())
            }
            Err(_) => {
                self.refused += 1;
                Err(quota)
            }
        }
    }
}

/// Instantiate the generation and start it.
///
/// `None` when this boot selected no generation, which is every boot with no
/// `f.root=` on its command line and is the ordinary case for the fault and
/// datapath boots. The caller falls back to the row loop it ran before RFC 0094.
///
/// A [`Refusal`] does not fail the boot either. It is reported on the board as
/// an ordinal and the caller carries on, because a malformed module must not turn
/// a machine that boots into one that does not — `docs/booting-on-hardware.md`
/// makes every component file optional, and this is that argument one level up.
pub fn assemble(
    board: &crate::routing::Board,
    control: &f_ring::adopt::Client,
) -> Option<Assembled> {
    if board.module_at == 0 || board.module_len == 0 {
        return None;
    }
    let Ok(granted) = f_ring::device::Granted::at(board.module_at, board.module_len) else {
        return Some(Assembled { refusal: refused(&Refusal::Files), ..Assembled::default() });
    };
    // Borrowed, not copied. The frame mapped the module read-only into this
    // component's address space, so the bytes are already here and a `Vec` of
    // them would be a second copy of a thing nobody writes.
    let bytes = granted.bytes();
    let Ok(module) = f_abi::boot::Module::read(bytes) else {
        return Some(Assembled { refusal: refused(&Refusal::Files), ..Assembled::default() });
    };

    let mut assembly = match Assembly::instantiate(&board.root, bytes) {
        Ok(assembly) => assembly,
        Err(why) => {
            return Some(Assembled { refusal: refused(&why), ..Assembled::default() });
        }
    };

    // An **empty** bus, deliberately. A component cannot scan PCI — the frame
    // owns configuration space and the remapping unit — so binding by declared
    // part is something this component can decide and not something it can
    // discover. `xtask/src/generation.rs` makes the same call for the host-side
    // instantiation and for the same reason.
    //
    // The consequence is stated rather than hidden: every member that declares a
    // `[[device]]` is left `NoDevice`, which `f_assembler::start::start` counts
    // as `absent` and does not treat as a failure. The frame binds those devices
    // itself when it fills their places, so the topology's account of them is
    // narrower than the machine's. Widening it needs the frame to tell this
    // component what it found, which is a board field and a later increment.
    let bus = Bus::new();
    let _ = f_assembler::bind::bind(&mut assembly, &bus);

    let mut spawning = Spawning::new(module, control, board);
    let report = f_assembler::start::start(&mut assembly, &mut spawning);
    let digest = f_assembler::render::digest(&assembly);
    // Rendered and dropped. The digest is what crosses the board; the rendering
    // is what the digest is over, and holding it would be this component keeping
    // a copy of something it has already summarised.
    let rendered: Vec<u8> = f_assembler::render::topology(&assembly);
    drop(rendered);

    let mut filled = [false; crate::routing::PLACES_MAX];
    for (slot, (_, taken)) in filled.iter_mut().zip(spawning.rows.iter()) {
        *slot = *taken;
    }
    Some(Assembled {
        digest,
        report,
        submitted: spawning.submitted,
        skipped: spawning.skipped,
        refusal: 0,
        filled,
    })
}

/// A refusal as an ordinal, plus one, so that zero means *there was none*.
fn refused(why: &Refusal) -> u64 {
    // Matched rather than cast, because `Refusal` carries payloads and a
    // discriminant read off one of those would move the day a variant gains a
    // field. The order is the declaration's.
    match why {
        Refusal::Module(_) => 1,
        Refusal::Tree(_) => 2,
        Refusal::Root => 3,
        Refusal::Files => 4,
        Refusal::Component(_, _) => 5,
        Refusal::Content(_) => 6,
        Refusal::Named(_) => 7,
        Refusal::Unrouted(_, _) => 8,
        Refusal::Unsupplied(_, _) => 9,
        Refusal::Cycle(_) => 10,
    }
}
