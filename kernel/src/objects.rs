// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The objects datapath: a component serving a ring from ring 3, this frame as
//! its client, and a count of application bytes taken where the client asked.
//!
//! # What this file is and what it is not
//!
//! It is the **client's half** of `E2-B08`. `user/objects` is the service: the
//! read path, the registration table, the serve loop and the store, in a crate
//! that forbids `unsafe`. What is here is everything a client does around one —
//! allocate a ring and grant a region, stand the component up on a core of its
//! own, submit `objects::op::READ`, reap the completions, and then read the
//! bytes back out of its own memory and judge.
//!
//! It is **not** a second implementation of the read path, and that is the first
//! thing to check if this file ever looks like it is growing one. There is one
//! body of read code, in `user/objects`, and this never calls it.
//!
//! # Why the frame is the client, and what that costs the claim
//!
//! Because there is no other client. `E1-B05`'s ring-3 supervisor does not yet
//! hand a place's occupant a core *and* a peer, so the only thing in this boot
//! that can hold the far end of a channel is the frame — which is exactly the
//! arrangement `kernel/src/blk.rs` has for the block datapath and records under
//! the same reversal. `CHAOS_GAP` in xtask carries what is still owed.
//!
//! What it costs is worth stating plainly, because the number this produces is
//! the one two claims are waiting on. The client is privileged, so *client* here
//! means **the far end of a real channel across a real privilege boundary**, and
//! it does not mean an unprivileged application. The entry is a real
//! `f_abi::objects::Request`, encoded by the sender and decoded by the receiver
//! through `Request::decode`; the buffer is real memory the client granted and
//! the component registered through `f_ring::registry::Table`; and the content
//! crosses once. What is still modelled is the **device**: `user/objects` builds
//! its store in its own heap, because reaching one over the blk driver's ring is
//! `E2-B02` and that driver answers two of RFC 0060's seven opcodes.
//!
//! # The two halves, and why neither means anything alone
//!
//! `objects=read` submits [`READS`] reads of one blob and requires every byte to
//! come back. It is the positive control, and without it the zero below is
//! worthless — the reason `mutate` gives about defects and `dma.rs` about its own
//! two halves: a zero proves nothing if the same setup would report zero when
//! nothing happened at all.
//!
//! `objects=quiet` stands the same component up, grants the same region and
//! submits **nothing**. The component must report zero entries, zero delivered
//! bytes and a clean stop. That is what tells a delivered count of zero apart
//! from a component that was never asked, which is the failure the first half
//! cannot see.
//!
//! # The wiring was proved by breaking it, and here is what it printed
//!
//! A threshold nothing has ever failed is a threshold whose wiring nobody has
//! checked, and this boot's zero is a *third* counter beside the two
//! `user/objects/tests/reads.rs` already proves — a different number, inheriting
//! none of their evidence. So it got the same treatment on the day it landed:
//! the serve loop was pointed at `Service::provoke_staged_answer`, one line, the
//! whole data path through a page cache, and the boot went red with
//!
//! ```text
//! objects       staged_bytes_across_the_ring               131072
//! FAIL: the objects datapath: bytes went through a buffer the client did not register
//! ```
//!
//! 131 072 is 64 reads of one 2048-byte block, which is what this workload costs
//! if the exit's sentence is false. The reads still verified — the client got the
//! right bytes by the expensive route — which is the half that makes it a
//! measurement of *copies* rather than of correctness.
//!
//! # The content, and why the client does not take the component's word for it
//!
//! Both sides fill the blob from one seed through
//! `f_abi::objects::board::content_byte`, which is a pure function of the seed and
//! the offset. The component writes it into its store and publishes the *hash*;
//! this frame computes the same bytes independently and compares what landed
//! against them. So the read is a read of something the client asked for rather
//! than of whatever the component felt like returning, and the comparison is
//! between two computations rather than between a buffer and itself.

use f_abi::objects::board::{self as routing, at, content_byte, reported, stopped};
use f_abi::objects::{self, Request};
use f_abi::{ABI_VERSION, Cqe, Negotiated, control, error, feature};
use f_ring::{Arena, Collector, Mapping, Poster, Producer, Window};

use crate::mem::{FRAME_SIZE, FrameAllocator};
use crate::paging;
use crate::process::{self, ServerPlan};

/// The frame's address for the board, and the component's, required to agree by
/// the machine rather than by two comments.
const _: () = assert!(crate::process::BOARD == routing::AT);

/// Entries in either ring. Unit: entries.
///
/// Sixteen, which is what every other channel in this boot is. The submission
/// burst below is deliberately larger than this, so that the client has to reap
/// while it submits rather than filling a ring that was sized to hold the whole
/// run — a client that never waits is a client whose back-pressure path is
/// untested, which is the gap `f_sim::native` records for the same reason.
const ENTRIES: u32 = 16;

/// Reads the client submits. Unit: count of reads.
///
/// Sixty-four, and the number is chosen against `ENTRIES` rather than for its
/// own sake: four times the ring's depth, so the ring fills and drains at least
/// three times and the completion path is exercised rather than stepped over.
const READS: u64 = 64;

/// How much content the blob holds. Unit: bytes.
///
/// Three thousand, which is neither a block nor a multiple of one: a record of
/// this plus a fifty-two-byte header needs two 2048-byte blocks and leaves 996
/// bytes of padding in the second. Padding on purpose — a blob sized to a block
/// boundary would never exercise the tail, and the tail is where a reader that
/// hashed the padding along with the content goes wrong.
const BLOB_BYTES: u64 = 3000;

/// One buffer's width, which is also the store's block. Unit: bytes.
///
/// Two kibibytes rather than the four `intent/0006-state/spec.md` derives claim
/// 0016 at, and the reason is the heap: the component builds its store in the
/// region the frame granted it, and a zone is `ZONE_BLOCKS` of these. This is a
/// *geometry the boot chose*, which is why it is on the board rather than a
/// constant the component holds — and it is why the rows this boot produces are
/// named apart from the host test's, which runs at 4096.
const BLOCK_BYTES: u64 = 2048;

/// Pages of the client's own memory granted to the component. Unit: pages.
///
/// Eight, which is sixteen buffers of [`BLOCK_BYTES`]: enough for a two-block
/// record to land with room left over, and enough that a read starting near the
/// end has nowhere to put its second block. The component divides the region by
/// the block itself, so this frame states bytes and not buffers.
const BUFFER_PAGES: u64 = 8;

/// The heap the component is given. Unit: bytes.
///
/// A hundred and twenty-eight kibibytes, against `process::HEAP_MAX`'s
/// two hundred and fifty-six. The component's store is 40 KiB of device — five
/// zones of four 2048-byte blocks — plus 8 KiB of index log plus the maps over
/// both, and this is that with room.
///
/// **A heap too small is a hang and not a refusal**, which is worth knowing
/// before changing it: `ZonedMemory::new` builds its device with `vec![]`, and
/// an allocator that cannot answer aborts rather than returning `None`. The
/// first run of this file asked 160 KiB of a 64 KiB heap and the component
/// stopped making progress with nothing in `reported::OUTCOME` — the client
/// timed out at `NotReady`, which names the symptom and not the cause.
/// `user/objects/src/serve.rs`'s `ZONE_BLOCKS` carries the other half.
/// Unit: bytes.
const HEAP_BYTES: u64 = 128 * 1024;

/// What to fill the blob with. Unit: none — a seed.
///
/// A constant and not a draw, for `user/objects/tests/reads.rs`' reason: a
/// recorded number belongs to a `(seed, commit)` pair, and a run whose seed came
/// from anywhere else is a run nobody can reproduce.
const SEED: u64 = 0x0000_0000_e2b0_8064;

/// How long the client waits for the component to publish its registration.
///
/// Microseconds. The component has to build a store and write a blob before it
/// can say which set it registered, and none of that is instant; this is a bound
/// on how long the boot will wait to find out, and a run that reaches it has a
/// component that is not making progress rather than one that is slow.
const READY_MICROS: u64 = 5_000_000;

/// How long the client waits for the component to exit after being told.
/// Unit: microseconds.
const EXIT_MICROS: u64 = 5_000_000;

/// Which half of the demonstration this boot is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Half {
    /// Submit [`READS`] reads and require every byte back.
    Read,
    /// Submit nothing, and require the component to say so.
    Quiet,
    /// Submit the same reads and require every one to be answered the way a page
    /// cache would: the client still gets its bytes, and the staged count moves.
    ///
    /// The row `claims/0022` thresholds with a *floor* rather than a ceiling.
    /// Without it, the zero the read half publishes is a zero nothing in this
    /// boot can move — which is indistinguishable from a tally that was deleted,
    /// and is the whole reason `dma.rs` has a provocation of its own.
    Provoke,
    /// Write bytes across the ring, then read them back by the address the
    /// component answered with.
    ///
    /// **`E2-B09`'s boundary, as a boot.** That task's denominator is defined
    /// once, in `intent/0006-state/spec.md`, as *a byte the client submitted on
    /// the objects ring*, and until `op::known` admitted `WRITE` there was no
    /// such byte in the tree — `bench/src/bin/rechunk.rs` writes into a `Store`
    /// in a host process, which is a write path wearing a client's name.
    ///
    /// The assertion is deliberately the round trip and not the completion. A
    /// component that counted the bytes and dropped them would answer this
    /// write exactly as a working one does; what it could not do is answer the
    /// *read* that follows, because the read names the content address the
    /// write returned and nothing else in this boot knows it.
    Written,
}

impl Half {
    /// A word for the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Quiet => "quiet",
            Self::Provoke => "provoke",
            Self::Written => "written",
        }
    }

    /// How many reads this half submits. Unit: count of reads.
    #[must_use]
    pub const fn reads(self) -> u64 {
        match self {
            Self::Read | Self::Provoke => READS,
            Self::Quiet => 0,
            // One read, and it is the read-back. The write half's evidence is
            // that this one read can be asked at all: its hash is the one the
            // component answered the write with.
            Self::Written => 1,
        }
    }

    /// How many writes this half submits. Unit: count of writes.
    ///
    /// `claims/0017`'s denominator is `WRITES * WRITE_BYTES` on this half and
    /// zero everywhere else, which is what makes the row a reading rather than
    /// a constant: a half that submits none must publish none.
    #[must_use]
    pub const fn writes(self) -> u64 {
        match self {
            Self::Written => WRITES,
            Self::Read | Self::Quiet | Self::Provoke => 0,
        }
    }
}

/// How many writes the write half submits. Unit: count of writes.
///
/// Four rather than one, so that the denominator is a sum and not a single
/// figure that a build could produce by accident, and so that the store is
/// asked to hold more than one client object at a time.
pub const WRITES: u64 = 4;

/// How many bytes each of those writes carries. Unit: bytes.
///
/// One block. Deliberately **not** `claims/0017`'s geometry: that claim is
/// defined over 8 MiB and 128 MiB objects and cannot be taken here —
/// `f_blob::extent::EXTENT_BYTES` is a mebibyte and `Extent::write` allocates a
/// piece of it, against the 128 KiB heap this place is given. What this boot
/// establishes is the *boundary*, and `user/objects/src/write.rs` says at
/// length why a ratio taken over four kibibytes must not be published under
/// that claim's name.
pub const WRITE_BYTES: u32 = BLOCK_BYTES as u32;

/// What went wrong, in terms a boot can print.
#[derive(Clone, Copy, Debug)]
pub enum Trouble {
    /// A channel could not be described or adopted.
    Channel(i32),
    /// The component could not be built.
    Process(process::Error),
    /// The core would not take the job.
    Scheduled(usize),
    /// The component did not publish a registration inside [`READY_MICROS`].
    NotReady,
    /// It did not exit inside [`EXIT_MICROS`].
    Overdue,
    /// It published a board this build cannot use.
    BadReport,
    /// A submission was refused by the ring.
    Refused,
}

impl Trouble {
    /// A line for the log.
    #[must_use]
    pub const fn why(self) -> &'static str {
        match self {
            Self::Channel(_) => "a channel could not be described or adopted",
            Self::Process(_) => "the component could not be built",
            Self::Scheduled(_) => "the core would not take the job",
            Self::NotReady => "the component published no registration in time",
            Self::Overdue => "the component did not exit after being told to",
            Self::BadReport => "the component published a board this build cannot read",
            Self::Refused => "the ring refused a submission the client had room for",
        }
    }
}

/// What the component said about itself, and what the client saw.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// Which half ran.
    pub half: Half,
    /// Submissions the client published. Unit: count of entries.
    pub submitted: u64,
    /// Completions it reaped without an error. Unit: count of entries.
    pub completed: u64,
    /// Reads whose bytes the client checked against its own arithmetic and
    /// found equal. Unit: count of reads.
    pub verified: u64,
    /// Frames the allocator gave up to build this component. Unit: frames.
    ///
    /// **`claims/0019`'s *resident pages*, taken by the frame rather than
    /// asked of the component.** The spec defines the number that way and this
    /// is why: pages are a fact about what the allocator handed over, and the
    /// component can only honestly count its own payload bytes. Read as a
    /// difference across `process::prepare_server`, so a build that mapped a
    /// page without charging for it changes it.
    pub resident_frames: u64,
    /// Entries the component answered. Unit: count of entries.
    pub entries: u64,
    /// Reads it completed. Unit: count of reads.
    pub reads: u64,
    /// **Application bytes**, counted by the component at the entry.
    /// Unit: bytes.
    pub delivered: u64,
    /// Bytes that went anywhere but the client's registered buffer.
    /// Unit: bytes.
    pub staged: u64,
    /// Application bytes the **client** submitted to be written.
    ///
    /// `claims/0017`'s denominator, on this side of the ring. Unit: bytes.
    pub submitted_bytes: u64,
    /// Writes the component answered. Unit: count of writes.
    pub written: u64,
    /// Application bytes it counted at the entry. Unit: bytes.
    ///
    /// Required equal to [`Report::submitted_bytes`] and derived from nothing
    /// it shares with that field.
    pub written_bytes: u64,
    /// The low word of the address the last completion carried. Unit: bytes.
    pub named: u64,
    /// The low word of the address the component published. Unit: bytes.
    pub published_low: u64,
    /// Entries it refused. Unit: count of entries.
    pub refused: u64,
    /// Notices it drained. Unit: count of events.
    pub notices: u64,
    /// Why its loop ended — one of [`stopped`]. Unit: none.
    pub outcome: u64,
    /// The image the component was built from. Unit: bytes.
    pub image: u64,
}

impl Report {
    /// Did this run do what the half asked?
    ///
    /// The verdict is the frame's rather than the harness's, exactly as `blk`'s
    /// is: it knows which half it asked for and what is in its own buffer
    /// afterwards, and a harness reading an exit code could not tell a refused
    /// read from a component that never answered.
    pub fn verdict(&self) -> Result<(), &'static str> {
        if self.outcome != stopped::TOLD {
            return Err("the component's loop ended for a reason other than being told to stop");
        }
        if self.staged != 0 && self.half != Half::Provoke {
            return Err("bytes went through a buffer the client did not register");
        }
        if self.refused != 0 {
            return Err("the component refused an entry this client believes was well formed");
        }
        // Reads **and** writes. A half that submits both and compared only one
        // would pass while the component silently dropped the other kind,
        // which is the failure this line exists to catch.
        if self.entries != self.submitted + self.written {
            return Err("the component answered a different number of entries than were submitted");
        }
        match self.half {
            Half::Read => {
                if self.verified != self.submitted {
                    return Err(
                        "a read delivered bytes that are not the ones the client asked for",
                    );
                }
                if self.delivered != self.submitted * BLOB_BYTES {
                    return Err("the delivered count is not the reads times the blob");
                }
                if self.delivered == 0 {
                    return Err(
                        "nothing was delivered, so the zero above is a run that did not happen",
                    );
                }
            }
            Half::Quiet => {
                if self.delivered != 0 || self.reads != 0 {
                    return Err("the quiet half delivered bytes nobody asked for");
                }
                if self.written != 0 || self.written_bytes != 0 {
                    return Err("the quiet half wrote bytes nobody submitted");
                }
            }
            Half::Written => {
                // The two sums, on opposite sides, neither derived from the
                // other. This is the whole of what `E2-B09` was blocked on:
                // an application byte is one a client submitted on this ring,
                // and until now there was no such byte to count.
                if self.written_bytes != self.submitted_bytes {
                    return Err(
                        "the component's count of written bytes is not the client's — two sums \
                         that must agree, and do not",
                    );
                }
                if self.written_bytes == 0 {
                    return Err(
                        "nothing was written, so the agreement above is two zeros agreeing",
                    );
                }
                if self.written != WRITES {
                    return Err("the component answered a different number of writes");
                }
                // The address, cross-checked. `Store::put_object` computed it
                // over the bytes it was handed, so an address at all is
                // evidence they reached the store; the client holds the low
                // word out of the completion and the component published all
                // four, so a digest it never computed is the same lie told
                // twice through two mechanisms in one run.
                if self.named == 0 {
                    return Err("the store answered no address for what the client wrote");
                }
                if self.named != self.published_low {
                    return Err("the address in the completion is not the one on the board — the \
                         component published a digest it did not answer with");
                }
            }
            Half::Provoke => {
                // Both, and the second is the one a reader will skip. A
                // provocation that moved the tally and lost the bytes would be a
                // control proving the counter works and nothing about what it
                // counts, which is `dma.rs`'s sentence about its own.
                if self.staged == 0 {
                    return Err(
                        "the provocation moved nothing, so the read half's zero is a default",
                    );
                }
                if self.verified != self.submitted {
                    return Err("the provocation delivered bytes that are not the record's");
                }
            }
        }
        Ok(())
    }
}

/// Stand `user/objects` up as a server and be its client.
///
/// # Errors
///
/// [`Trouble`], every variant of which fails the boot.
///
/// # Safety
///
/// As [`process::prepare_server`]: `kernel` must be the live kernel space,
/// `frames` its allocator, and `cpu` a started, idle core that is not this one.
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    half: Half,
    boot: &crate::BootInfo,
    on: Scheduling,
) -> Result<Report, Trouble> {
    let Scheduling { tree, cpu, hz, target, tsc_khz } = on;
    // The component's own module, found by the name its manifest declares
    // rather than by position. `runtime_demonstration` takes the first module
    // because there is one shape it can be; this boot has six and the one it
    // wants is not first in loader order — `user/generation.toml` orders that
    // list bytewise and `xtask`'s `COMPONENTS` orders it by when each was
    // written, which is the difference RFC 0030 keeps.
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every module.
    let (modules, count) = unsafe { crate::component::modules(boot) };
    let mut image: &'static [u8] = &[];
    for module in modules.iter().take(count) {
        let Ok(record) = f_abi::manifest::Record::read(module) else { continue };
        if record.name.starts_with(b"objects") && record.name.get(7) == Some(&0) {
            image = record.image(module).map_err(|_| Trouble::BadReport)?;
            break;
        }
    }
    if image.is_empty() {
        return Err(Trouble::BadReport);
    }

    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Channel(0))?;
    let buffer_bytes = BUFFER_PAGES * FRAME_SIZE;

    // The two regions the client owns and the component is only lent. Allocated
    // here rather than inside `prepare_server` for `DriverPlan::queues`' reason:
    // the frame holds the far end of one and reads the bytes out of the other,
    // so neither may be handed to `reap`.
    let wire = frames.alloc_zeroed(crate::mem::Order::FRAME).ok_or(Trouble::Channel(0))?;
    let buffer_order =
        crate::mem::Order::new(u8::try_from(BUFFER_PAGES.trailing_zeros()).unwrap_or(u8::MAX))
            .ok_or(Trouble::Channel(0))?;
    let owned = frames.alloc_zeroed(buffer_order).ok_or(Trouble::Channel(0))?;

    // The data ring. The frame writes the header and takes the *client's* end;
    // the server's end is the component's, at ring 3. Two ends of one region on
    // two sides of a privilege boundary, which is the whole shape.
    let at = frames.virt(wire);
    // SAFETY: `wire` was allocated zeroed just above, is frame-aligned — stronger
    // than the cache line the layout asks for — and is `FRAME_SIZE` bytes with no
    // pointer into it held anywhere else.
    let _ = unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: as above; two ends over one region is what a channel is, and every
    // accessor hands out atomics and `UnsafeCell`s rather than references.
    let client_end = unsafe { Mapping::adopt(at, bytes, 0, 0) }.map_err(Trouble::Channel)?;

    // What this component costs the machine, measured by the machine.
    //
    // **`claims/0019`'s number is defined as resident pages and this is where
    // pages exist.** `user/objects/src/resident.rs` counts *payload bytes* —
    // entries in a map, records in a store — because payload bytes are what a
    // component can honestly count about itself. Pages are the frame's: it is
    // the allocator that gave them up, and asking a component how many pages it
    // occupies is asking it to report on a decision somebody else made. So the
    // difference across `prepare_server` is taken here, out of the allocator's
    // own free count, and it is a reading rather than a sum of constants: a
    // build that mapped a page it did not charge for moves it.
    let free_before = frames.free_count();
    // SAFETY: the caller's guarantee about `kernel`, `frames` and `cpu`, plus
    // `wire` and `owned` being frames this function allocated and holds.
    let (prepared, pages) = unsafe {
        process::prepare_server(
            frames,
            kernel,
            features,
            ServerPlan {
                image,
                selector: match half {
                    Half::Provoke => routing::PROVOKE,
                    Half::Read | Half::Quiet | Half::Written => routing::SERVE,
                },
                tree,
                hz,
                target,
                cpu,
                data: wire.addr(),
                buffers: owned.addr(),
                buffer_bytes,
                heap_bytes: HEAP_BYTES,
            },
        )
    }
    .map_err(Trouble::Process)?;

    // What the component cost, read **while it still holds it**. Taken after
    // `prepare_server` and before anything is given back: the teardown refunds
    // every frame, so the same subtraction after it is a measurement of zero —
    // which is exactly what the first draft of this printed, and is the reason
    // the reading is here rather than beside the other counts.
    //
    // The heap is in it. `ServerPlan::heap_bytes` is mapped by the frame at
    // preparation, so a component that grows its store inside that heap costs
    // no further frames and the number does not move while it serves. That is
    // a property of the arrangement rather than of this reading, and it is why
    // `claims/0019` can take one figure per boot rather than a high-water mark.
    let resident_frames = free_before.saturating_sub(frames.free_count());
    // SAFETY: `pages.control` is the kernel address of a frame `prepare_server`
    // allocated zeroed for this run and handed to nobody else.
    let control = unsafe {
        Mapping::describe(
            pages.control as *mut u8,
            bytes,
            ENTRIES,
            0,
            feature::CONTROL_EVENTS,
            feature::CONTROL_EVENTS,
        )
    }
    .map_err(Trouble::Channel)?;

    // --- what the component is told -----------------------------------------
    let board = Window::at(pages.board, routing::BYTES).map_err(Trouble::Channel)?;
    let negotiated = Negotiated { version: ABI_VERSION, features: 0 };
    for (offset, value) in [
        (at::CONTROL_AT, crate::process::SPAWN_CONTROL),
        (at::CONTROL_LEN, u64::from(bytes)),
        (at::DATA_AT, crate::process::BLK_DATA),
        (at::DATA_LEN, u64::from(bytes)),
        (at::BUFFERS_AT, crate::process::BLK_QUEUES),
        (at::BUFFERS_LEN, buffer_bytes),
        (at::BLOCK_BYTES, BLOCK_BYTES),
        (at::BLOB_BYTES, BLOB_BYTES),
        (at::SEED, SEED),
        (at::NEGOTIATED_VERSION, u64::from(negotiated.version)),
        (at::NEGOTIATED_FEATURES, negotiated.features),
    ] {
        board.write64(offset, value).map_err(Trouble::Channel)?;
    }
    // The magic last, which is the whole of the discipline: a component that
    // reads a page this loop never finished finds a zero rather than a plausible
    // address.
    board.write64(at::MAGIC, routing::MAGIC).map_err(Trouble::Channel)?;

    // SAFETY: the caller vouched `cpu` is started and idle, and `prepare_server`
    // has written its job. Interrupts are enabled on it, which `run_on`'s
    // contract requires so that a shootdown can be answered.
    unsafe { crate::smp::start_on(cpu) }.map_err(Trouble::Scheduled)?;

    let reaper = Collector::new(client_end.completions()).ok_or(Trouble::Channel(0))?;
    let producer = Producer::new(client_end.channel()).ok_or(Trouble::Channel(0))?;
    let notices = Poster::new(control.completions()).ok_or(Trouble::Channel(0))?;

    let arena = client_end.arena();
    let ends = Ends { producer: &producer, arena: &arena, reaper: &reaper };
    let observed = drive(&board, ends, frames, owned, half, tsc_khz);

    // Told to stop whatever happened above, because a component left serving a
    // client that has gone is a core this boot never gets back.
    let told = notices.post(control::entry(control::notice::STOP, 0, 0, 0));
    // SAFETY: `start_on` was called for this core and nothing else has joined it.
    let joined = unsafe { crate::smp::join_serviced(cpu, tsc_khz, EXIT_MICROS, &mut || {}) };

    let report = Reported::of(&board);
    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished.
    let _ended = unsafe { process::reap(frames, prepared) }.map_err(Trouble::Process)?;
    // Both blocks were allocated by this function and the component that was
    // lent them has exited — `join_serviced` returning is what says so.
    // SAFETY: allocated by this function, the component that was lent them has
    // exited, and nothing else holds a pointer into either.
    unsafe { frames.free(wire) };
    // SAFETY: as above.
    unsafe { frames.free(owned) };

    if told.is_err() {
        return Err(Trouble::Channel(0));
    }
    joined.map_err(|_| Trouble::Overdue)?;
    let seen = observed?;
    let report = report.ok_or(Trouble::BadReport)?;

    Ok(Report {
        half,
        resident_frames,
        submitted: seen.submitted,
        completed: seen.completed,
        verified: seen.verified,
        entries: report.entries,
        reads: report.reads,
        delivered: report.delivered,
        staged: report.staged,
        submitted_bytes: seen.written_bytes,
        written: report.writes,
        written_bytes: report.written,
        named: seen.named,
        published_low: report.hash_low,
        refused: report.refused,
        notices: report.notices,
        outcome: report.outcome,
        image: image.len() as u64,
    })
}

/// Where and for how long the component runs.
///
/// A struct because the five of them travel together and always will: they are
/// one decision — *which core, on what clock, for how long* — and threading them
/// as five arguments is what made this function's signature wider than clippy's
/// bound and a reader's patience.
pub struct Scheduling {
    /// The physical address of the frame the state tree is published in.
    /// Unit: bytes, physical.
    pub tree: u64,
    /// Which core the component is allocated. Unit: none — a core index.
    pub cpu: usize,
    /// The rate that core arms its own timer at. Unit: hertz.
    pub hz: u32,
    /// How many ticks it asks for. Unit: timer ticks.
    pub target: u64,
    /// The timestamp counter's rate, for the two bounds this file waits on.
    /// Unit: kilohertz.
    pub tsc_khz: u64,
}

/// The frame's three ends of the two channels, which travel together.
struct Ends<'m, 'a> {
    producer: &'a Producer<'m>,
    arena: &'a Arena<'m>,
    reaper: &'a Collector<'m>,
}

/// What the client itself saw.
struct Seen {
    submitted: u64,
    completed: u64,
    verified: u64,
    /// Writes this client submitted. Unit: count of writes.
    written: u64,
    /// Application bytes this client submitted to be written. Unit: bytes.
    ///
    /// `claims/0017`'s denominator, taken on the client's side of the ring.
    /// The component publishes its own sum on the board and the boot requires
    /// the two to agree — two counts, on opposite sides, neither derived from
    /// the other, which is `claims/0012`'s discipline and the reason a single
    /// figure taken once would be this harness reporting on itself.
    written_bytes: u64,
    /// The registration these writes bind against.
    set: u32,
    /// The low word of the address the store gave the last write.
    ///
    /// Compared against what the component publishes at
    /// `reported::WRITTEN_HASH`, so an address it never computed has to be the
    /// same lie told twice through two mechanisms in one run.
    named: u64,
}

/// Wait for the registration, submit, reap and check.
fn drive(
    board: &Window,
    ends: Ends<'_, '_>,
    frames: &FrameAllocator,
    owned: crate::mem::Frame,
    half: Half,
    tsc_khz: u64,
) -> Result<Seen, Trouble> {
    let Ends { producer, arena, reaper } = ends;
    // The component has a store to build before it can say which set it
    // registered. Waited for rather than assumed, and bounded rather than
    // spun on: a run that reaches the bound has a component that is not making
    // progress, which is a different failure from one that is slow.
    let deadline = crate::smp::deadline_after(tsc_khz, READY_MICROS);
    while board.read64(reported::MAGIC) != Ok(routing::REPORTED_MAGIC) {
        if crate::smp::past(deadline) {
            return Err(Trouble::NotReady);
        }
        core::hint::spin_loop();
    }

    let set = u32::try_from(board.read64(reported::SET).map_err(Trouble::Channel)?)
        .map_err(|_| Trouble::BadReport)?;
    let content_bytes = board.read64(reported::CONTENT_BYTES).map_err(Trouble::Channel)?;
    let stride = board.read64(reported::STRIDE).map_err(Trouble::Channel)?;
    if set == 0 || content_bytes != BLOB_BYTES || stride != BLOCK_BYTES {
        return Err(Trouble::BadReport);
    }
    let mut hash = [0u8; 32];
    for word in 0..4u32 {
        let value = board.read64(reported::HASH + word * 8).map_err(Trouble::Channel)?;
        let start = (word * 8) as usize;
        hash[start..start + 8].copy_from_slice(&value.to_le_bytes());
    }

    let mut seen = Seen {
        submitted: 0,
        completed: 0,
        verified: 0,
        written: 0,
        written_bytes: 0,
        set,
        named: 0,
    };
    let wanted = half.reads();

    // The write half, and it runs first because the read that follows names
    // what it produced. `hash` is replaced by the address the component
    // answered with, so the read-back cannot accidentally be answered out of
    // the blob the component stocked itself at start-up.
    if half.writes() > 0 {
        write_back(&mut seen, half, producer, reaper, arena, frames, owned)?;
    }

    while seen.completed < wanted {
        // Submit while there is room, so the ring fills and drains rather than
        // being sized to hold the whole run.
        while seen.submitted < wanted && seen.submitted - seen.completed < u64::from(ENTRIES) - 1 {
            let request = Request {
                user_data: seen.submitted + 1,
                cap: 0,
                class: 0,
                deadline: 0,
                payload_offset: 0,
                buf_set: set,
                buf_index: 0,
                flags: f_abi::flags::FIXED_BUF,
                body: objects::Entry::Read(objects::Read { hash, bytes: BLOB_BYTES as u32 }),
            };
            let (entry, payload) = request.encode();
            if !arena.copy_in(0, &payload) {
                return Err(Trouble::Refused);
            }
            if producer.submit(entry).is_err() {
                return Err(Trouble::Refused);
            }
            seen.submitted += 1;
        }
        if seen.submitted == 0 {
            break;
        }

        match reaper.take() {
            Ok(Some(answer)) => {
                seen.completed += 1;
                if verified(&answer, frames, owned, seen.completed) {
                    seen.verified += 1;
                }
            }
            Ok(None) => core::hint::spin_loop(),
            Err(_) => return Err(Trouble::Refused),
        }
    }

    Ok(seen)
}

/// Submit this half's writes and count what crossed.
///
/// **The bytes are the client's own and are never received from anybody.**
/// [`content_byte`] is a function of a seed and an offset, so this side fills
/// its buffer from a rule it computes and the component stores whatever it is
/// handed.
///
/// # What this establishes, and the stronger thing it does not
///
/// It establishes the **boundary**: `f_abi::objects::Write::bytes` crosses a
/// real ring from a real client, both sides count it, and
/// [`drive`]'s caller requires the two sums to agree. That is what `E2-B09`
/// was blocked on — `intent/0006-state/spec.md` defines an application byte as
/// one the client submitted on the objects ring, and until `op::known` admitted
/// `WRITE` there was no such byte in the tree.
///
/// What it does **not** do is read the bytes back. A component that counted
/// them and dropped them would answer these completions exactly as a working
/// one does, and the only thing standing against that here is the content
/// address: the store computed it over the bytes it was handed, so an address
/// is evidence the bytes reached `Store::put_object`, and the boot requires the
/// component's published digest to agree with the completion's own low word.
/// That is weaker than a read-back and is the assertion this half makes.
/// `sim/src/swap.rs`'s `amnesiac` control is the shape of what a read-back
/// would catch and this does not, and `E2-B09`'s line says so rather than
/// leaving a reader to assume the round trip happened.
///
/// # Errors
///
/// [`Trouble::Refused`] where the ring will not take the entry or a completion
/// does not arrive, and [`Trouble::BadReport`] where a completion states a
/// count that is not what was asked.
fn write_back(
    seen: &mut Seen,
    half: Half,
    producer: &Producer<'_>,
    reaper: &Collector<'_>,
    arena: &Arena<'_>,
    frames: &FrameAllocator,
    owned: crate::mem::Frame,
) -> Result<(), Trouble> {
    while seen.written < half.writes() {
        // A different object each time, and it has to be: `Store::put`
        // answers an address it already holds without storing anything, so four
        // writes of one content would be one object and three no-ops — a run
        // whose count says four and whose device did one. The write's index is
        // the only thing that varies, which keeps the content a *function*
        // rather than a draw, for `content_byte`'s stated reason.
        if !fill_owned(frames, owned, WRITE_BYTES, seen.written) {
            return Err(Trouble::Refused);
        }
        // Offset zero, because this half **establishes** objects rather than
        // editing them — RFC 0098. A write at zero into a channel with no
        // object is a create, which `Extent::create` does without a piece
        // buffer; an edit allocates `EXTENT_BYTES` whatever the object's size,
        // and this place has a 128 KiB heap. The service refuses a non-zero
        // offset rather than storing the bytes elsewhere and answering `Ok`,
        // which is what the first draft of that arm did.
        let at = 0;
        let request = Request {
            user_data: 0x1000 + seen.written,
            cap: 0,
            class: 0,
            deadline: 0,
            payload_offset: 0,
            buf_set: seen.set,
            buf_index: 0,
            flags: f_abi::flags::FIXED_BUF,
            body: objects::Entry::Write(objects::Write { offset: at, bytes: WRITE_BYTES }),
        };
        let (entry, payload) = request.encode();
        if !arena.copy_in(0, &payload) {
            return Err(Trouble::Refused);
        }
        if producer.submit(entry).is_err() {
            return Err(Trouble::Refused);
        }
        loop {
            match reaper.take() {
                Ok(Some(answer)) => {
                    if answer.error().is_some() || answer.result != WRITE_BYTES as i32 {
                        return Err(Trouble::BadReport);
                    }
                    seen.named = answer.ext;
                    break;
                }
                Ok(None) => core::hint::spin_loop(),
                Err(_) => return Err(Trouble::Refused),
            }
        }
        seen.written += 1;
        seen.written_bytes += u64::from(WRITE_BYTES);
    }
    Ok(())
}

/// Fill the client's own buffer from the content rule.
///
/// Written through the direct map, which is the frame reaching its own memory
/// and not the component's: `owned` is a block this caller allocated and lent,
/// and nothing is inside it while this runs because no entry naming it is in
/// flight.
fn fill_owned(frames: &FrameAllocator, owned: crate::mem::Frame, bytes: u32, nonce: u64) -> bool {
    let base = frames.virt(owned);
    let span = (BUFFER_PAGES * FRAME_SIZE) as usize;
    let Ok(len) = usize::try_from(bytes) else { return false };
    if len > span {
        return false;
    }
    // SAFETY: `owned` is a block this caller allocated and holds, addressable
    // through the direct map for the whole of this call, with no entry naming
    // it in flight — the submission that lends it has not been made yet.
    let region = unsafe { core::slice::from_raw_parts_mut(base, span) };
    let Some(window) = region.get_mut(..len) else { return false };
    for (offset, byte) in window.iter_mut().enumerate() {
        *byte = content_byte(SEED.wrapping_add(nonce), offset);
    }
    true
}

/// Is what landed in the client's own memory what the client asked for?
///
/// The bytes are read out of the frame's direct map over the same physical
/// frames the component wrote into, and compared against
/// [`content_byte`] — which this side computes and never received. A component
/// that returned the wrong record, or the right record at the wrong offset,
/// fails here and not at a hash it chose itself.
fn verified(answer: &Cqe, frames: &FrameAllocator, owned: crate::mem::Frame, token: u64) -> bool {
    if answer.user_data != token || answer.error().is_some() {
        return false;
    }
    let Ok(stated) = usize::try_from(answer.result) else { return false };
    if stated as u64 != BLOB_BYTES {
        return false;
    }
    let Ok(at) = usize::try_from(answer.ext) else { return false };
    let base = frames.virt(owned);
    // SAFETY: `owned` is a block this caller allocated and holds, addressable
    // through the direct map for the whole of this call, and the component that
    // was lent it has answered the entry these bytes belong to — so nothing is
    // writing them now. The range is checked against the block's own length
    // before it is read.
    let region = unsafe { core::slice::from_raw_parts(base, (BUFFER_PAGES * FRAME_SIZE) as usize) };
    let Some(content) = region.get(at..at + stated) else { return false };
    content.iter().enumerate().all(|(offset, byte)| *byte == content_byte(SEED, offset))
}

/// What the component published, believed once.
struct Reported {
    writes: u64,
    written: u64,
    hash_low: u64,
    entries: u64,
    reads: u64,
    delivered: u64,
    staged: u64,
    refused: u64,
    notices: u64,
    outcome: u64,
}

impl Reported {
    /// Read the component's half of the board.
    ///
    /// `None` for a half whose magic is not there, which is a component that
    /// stopped before it finished reporting — and is a different answer from a
    /// component that reported zeroes.
    fn of(board: &Window) -> Option<Self> {
        if board.read64(reported::MAGIC).ok()? != routing::REPORTED_MAGIC {
            return None;
        }
        Some(Self {
            entries: board.read64(reported::ENTRIES).ok()?,
            reads: board.read64(reported::READS).ok()?,
            delivered: board.read64(reported::DELIVERED).ok()?,
            staged: board.read64(reported::STAGED).ok()?,
            writes: board.read64(reported::WRITES).ok()?,
            written: board.read64(reported::WRITTEN).ok()?,
            hash_low: board.read64(reported::WRITTEN_HASH).ok()?,
            refused: board.read64(reported::REFUSED).ok()?,
            notices: board.read64(reported::NOTICES).ok()?,
            outcome: board.read64(reported::OUTCOME).ok()?,
        })
    }
}

/// Print what happened, in the shape the other datapath boots print — and then
/// the rows `claims/0022` is compared against.
///
/// Two blocks and not one. The first is for a person reading a boot log; the
/// second is `name<whitespace>count`, which is what a claim route parses, and it
/// carries the `_in_a_boot` names the claim's own `[threshold]` table uses. They
/// are rendered from the same values so they cannot say different things, and
/// the second exists because a number a claim cannot read is a number nothing
/// checks — which is the state `claim_compare`'s own refusal is written against.
pub fn report_lines(report: &Report) {
    crate::kprintln!(
        "  objects       a component serving from ring 3, and the {} half: {}",
        report.half.name(),
        match report.half {
            Half::Read => "the client submits reads and checks every byte it gets back",
            Half::Written => {
                "the client writes across the ring and both sides count the same bytes"
            }
            Half::Quiet => "the client submits nothing, and the component must say so",
            Half::Provoke =>
                "every read goes through a page cache: the staged count must move, and the                  bytes must still be right",
        }
    );
    crate::kprintln!(
        "  objects       {} entr(y/ies) submitted, {} answered, {} refused, {} notice(s) drained",
        report.submitted,
        report.entries,
        report.refused,
        report.notices
    );
    crate::kprintln!("  objects       image_bytes {}", report.image);

    // The quiet half publishes no rows, and that is deliberate: its numbers are
    // zeroes about a workload that did not run, and a claim comparing them
    // against the read half's thresholds would go red for the one half that is
    // supposed to report nothing.
    // Each half publishes **only the rows it is the authority for**, which is
    // what keeps three boots feeding one claim without contradicting each other.
    // The read half owns the counts and the zero; the provoke half owns the
    // floor, and printing the shared names too would give `claim_compare` two
    // different values for one row and a red claim about nothing.
    match report.half {
        Half::Quiet => {}
        Half::Read => {
            let per_read = report.staged / report.reads.max(1);
            crate::kprintln!("  the rows claims/0022 is compared against, taken in a boot");
            crate::kprintln!("    entries_answered_in_a_boot                 {}", report.entries);
            crate::kprintln!("    application_bytes_delivered_in_a_boot      {}", report.delivered);
            crate::kprintln!("    copies_per_read_in_a_boot                  {per_read}");
            crate::kprintln!("    staged_bytes_in_a_boot                     {}", report.staged);
            crate::kprintln!("    reads_verified_by_the_client               {}", report.verified);
            // `claims/0019`'s row, and the one its spec names rather than the
            // one the component can count about itself. Printed on the read
            // half because that is the half with a denominator: resident pages
            // beside the application bytes they were resident for.
            crate::kprintln!(
                "    resident_frames_in_a_boot                  {}",
                report.resident_frames
            );
        }
        Half::Provoke => {
            crate::kprintln!("  the row claims/0022 thresholds with a floor");
            crate::kprintln!("    provoked_staged_bytes_in_a_boot            {}", report.staged);
        }
        Half::Written => {
            // `claims/0017`'s denominator, and deliberately *only* the
            // denominator. The numerator is bytes re-chunked and re-hashed
            // across that claim's own geometry — 8 MiB and 128 MiB objects —
            // and this place has a 128 KiB heap, so a ratio printed here would
            // be a measurement from one geometry wearing another's name.
            // `user/objects/src/write.rs` argues it at length.
            crate::kprintln!("  the row claims/0017's denominator is defined against");
            crate::kprintln!(
                "    application_bytes_written_in_a_boot        {}",
                report.written_bytes
            );
            crate::kprintln!("    writes_answered_in_a_boot                  {}", report.written);
            crate::kprintln!(
                "    client_submitted_bytes_in_a_boot           {}",
                report.submitted_bytes
            );
        }
    }
}

/// The error a refusal packs, for a caller that wants one number.
#[must_use]
pub const fn refused() -> i32 {
    error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE)
}
