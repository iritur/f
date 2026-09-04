// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The display server: the one entry it answers, the six commands that answer
//! is made of, and the counter that says the pixels went nowhere near this
//! component.
//!
//! # What a request names, and what it therefore cannot be
//!
//! A **registered buffer set and an index**, never an address and never a
//! payload. `user/virtio-gpu/manifest.toml` declares `payload = "registered"`
//! and this file is what makes that declaration a mechanism:
//! [`Registered::resolve`](f_ring::registry::Registered) answers a
//! [`Reach`](f_ring::registry::Reach), a `Reach` is an address and a length and
//! deliberately not a slice, and the address goes into a display command the
//! device reads out of. At no point does this component hold a reference to a
//! client's pixels, so at no point can it copy them.
//!
//! That is `user/virtio-blk/src/driver.rs`'s paragraph and
//! `user/virtio-net/src/driver.rs`'s, and it survives a third time unedited.
//! What follows is the part that does not survive, and none of it is what
//! E1-B03 predicted a third driver would find.
//!
//! # A display command is a request, which puts this driver back on blk's shape
//!
//! RFC 0051 listed five things the network driver could not reuse from the block
//! driver, and every one of them was the *receive direction*: an executor that
//! could not say *accepted, answer to follow*, a used element that had to be
//! read for its head, a wait with nothing owed, a teardown that owed clients
//! their buffers back, and a point past which a refusal is a wrong answer rather
//! than a smaller one.
//!
//! **Four of the five do not apply here.** Every virtio-gpu command is a request
//! the device owes a typed answer to, so [`Driver::execute`] answers where it
//! reads, one chain is outstanding at a time so the head is a constant, the wait
//! is bounded by a device that owes something, and there is nothing posted at
//! teardown to give back. This driver's *shape* is the block driver's. That is
//! the first useful thing E1-B04 says: the second driver's differences were
//! about receiving and not about being a second driver, which is a claim RFC
//! 0051 could not make and this one can.
//!
//! # The fifth one does apply, and it applies to something new
//!
//! *Find the point at which the device owns the buffer, and treat every refusal
//! below it as a bug* — RFC 0051's one sentence for a third driver. On both
//! other drivers that point is a **queue** event: `Queue::offer`'s publishing
//! store hands the device a descriptor, and the used element hands it back. The
//! ownership interval is one chain long.
//!
//! On a display it is neither. `RESOURCE_ATTACH_BACKING` gives the device a
//! guest address that it **keeps**: the chain that carried it completes
//! immediately, and the device goes on holding the mapping until a later,
//! separate `RESOURCE_DETACH_BACKING` takes it away. So the interval in which a
//! client's buffer belongs to the device is bounded by a *pair of commands* and
//! not by a chain, and it spans four chains that each complete successfully in
//! between. A driver that reasoned about ownership in terms of its queue — which
//! is what both other drivers do, correctly — would hand a client its buffer
//! back while a display controller was still entitled to read it.
//!
//! That is why [`Driver::perform`] sends `RESOURCE_DETACH_BACKING` as its last
//! command and why it is not optional, and it is the only place in this crate
//! where a refusal is answered after something has failed: a failure between the
//! attach and the detach is answered by trying the detach, and — if *that* fails
//! — by resetting the device, which blanks the screen. `crate::transport` argues
//! why a reset is otherwise refused, and this is the one thing worth blanking a
//! screen for: the alternative is telling a client its memory is its own again
//! while a display is reading it onto somebody's monitor.
//!
//! # What is minimal, and what each absence costs
//!
//! Six commands: create a two-dimensional resource, attach a client's buffer to
//! it as backing, transfer the pixels into it, set scanout zero to it, flush,
//! detach. The task's list of five is here plus the detach, which is not an
//! extra: without it the client never gets its buffer back, and the paragraph
//! above is why.
//!
//! What is left out, each with its cost:
//!
//! - **No `RESOURCE_UNREF`.** A resource is created per frame and never freed,
//!   so the display's memory grows by one frame per [`op::SHOW`] and its
//!   identifier space by one. [`RESOURCES_MAX`] is what keeps that from being
//!   unbounded, and it is a refusal rather than a wrap: a client that asks for a
//!   ninth frame is told `RESOURCE`/`QUOTA_EXHAUSTED` instead of quietly reusing
//!   an identifier the display still holds. `sim/src/gpu.rs` made the same
//!   choice for the same reason and says so — *a driver that freed as it went
//!   would never reach the display's limit, and the limit is the refusal worth
//!   modelling* — and the two arriving there independently is worth more than
//!   either of them arriving there. What it costs is that this driver cannot
//!   run for long, and the reason it is not fixed is a lifetime: a resource is
//!   freed when the surface it draws is dropped, and nothing in this system owns
//!   a surface yet.
//! - **No `GET_DISPLAY_INFO`.** This driver does not ask the display how large
//!   it is or how many scanouts it has. The geometry of a frame is the
//!   *client's*, carried in the entry, and scanout zero is assumed to exist. The
//!   cost is exact: on a machine whose display has no scanout zero the
//!   `SET_SCANOUT` is refused by the device, one round trip later than a driver
//!   that had asked would have refused it, and the client is told `DEVICE` with
//!   the display's own code rather than `ARGUMENT`.
//! - **No cursor.** `crate::queue::index::CURSOR` states the cost.
//! - **No `TRANSFER_FROM_HOST`, no reading the scanout back.** The 2D protocol
//!   has no such command, which is the fact that makes `cargo xtask gpu`'s
//!   observation what it is: the only way to find out what is on the screen is
//!   to look at it from outside the machine. RFC 0054.
//! - **One format, `B8G8R8X8_UNORM`.** [`FORMAT`] says why that one and what a
//!   second would cost.
//!
//! # The counter, and why there are two of them
//!
//! Unchanged from both other drivers, deliberately, because the property being
//! claimed is the same one. There is exactly one function in this crate that
//! moves bytes — [`stage`] — and it takes the tally it moves as an argument. The
//! data path never calls it, so [`Counters::copies`] is zero.
//! [`Driver::provoke_copy`] calls it against the driver's own control page, so
//! [`Counters::provoked`] is not. A build in which `stage` had been deleted, or
//! had stopped counting, would publish a zero in *both*, and
//! `cargo xtask lint-datapath` is what turns *exactly one* and *never on the
//! data path* from prose into a check with a fixture that breaks it.
//!
//! What is worth stating for this driver and not the last two: the zero here is
//! **easier** to hold than either of theirs, and pretending otherwise would be
//! dishonest. A block driver could be zero-copy by accident and a network
//! driver's receive path is the hard case — this component is not between the
//! device and the pixels at all, in either direction. What the zero is evidence
//! of here is not restraint but *structure*: there is no code path in this crate
//! through which a client's pixels could reach it, because the only thing this
//! crate ever learns about them is an address it hands to a device and takes
//! back.

use f_abi::buf::{Name, opcode};
use f_abi::deadline::{Admitted, Callee, Caller, Inherited};
use f_abi::{Cqe, Negotiated, Sqe, cflags, error, flags};
use f_ring::device::Region;
use f_ring::registry::{Domains, Refusal, Registered, Table, Transport as _};
use f_ring::{completion, refusal};

use crate::Trouble;
use crate::queue::{DESC_NEXT, DESC_WRITE, QUEUE_BYTES, QUEUE_SIZE, Queue};
use crate::transport::{Transport, Windows};

/// The opcodes this service answers on.
///
/// Numbered from one and not from zero, for R04 rather than taste:
/// `f_abi::op::NOP` is zero in the frame's own vocabulary, and an entry that
/// arrived here zeroed — a slot pulled off a free list, a peer that memset an
/// entry — would otherwise be a *show of buffer zero*. Zero names nothing here,
/// so a zeroed entry is refused.
///
/// **One opcode, and that is the whole protocol.** A reader who expects a verb
/// per display command — create, attach, scanout, transfer, flush — should read
/// [`Driver::perform`] and then this paragraph again: those five are a *device's*
/// vocabulary, and putting them on a ring would make every client of this
/// component a display driver. What a client of a display wants to say is *show
/// these pixels*, and the sequence that makes it happen is the thing this
/// component exists to know.
pub mod op {
    /// Put the pixels in the named buffer on scanout zero.
    ///
    /// The geometry is in `Sqe::ext` — `crate::driver::geometry` says why there
    /// rather than in a command of its own — and the length must be exactly what
    /// that geometry needs.
    pub const SHOW: u8 = 1;

    /// Is this an opcode this service implements?
    ///
    /// The negative answer is the one that matters: everything else is refused
    /// with `ARGUMENT/UNKNOWN_OPCODE` rather than being read as the nearest
    /// thing, which is R04 at the one place a client's mistake would otherwise
    /// become a picture nobody asked for.
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, SHOW)
    }
}

/// The pixel format every resource this driver creates is made of.
///
/// `VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM`, which is 2 in the specification's
/// numbering: four bytes per pixel, blue first in memory, and the fourth byte
/// ignored. One format and not a field on the entry, and the choice is argued
/// rather than defaulted.
///
/// It is the format every virtio-gpu implementation supports, because it is the
/// one a host framebuffer already is on the machines this device was written
/// for. A driver that let a client choose would be a driver whose refusals
/// depend on what the emulator was compiled with, which is the same argument
/// `crate::transport` makes about feature bits.
///
/// What it costs: a client that holds pixels in any other arrangement has to
/// rearrange them, and rearranging them is a copy — in the *client*, which is
/// where a copy belongs when somebody chose a layout, and not in this component,
/// which is the whole point of [`Counters::copies`] being zero. The day a client
/// legitimately holds another format, the field goes on the entry beside the
/// geometry and this constant becomes a default.
///
/// Unit: none — a `virtio_gpu_formats` ordinal.
pub const FORMAT: u32 = 2;

/// Bytes one pixel of [`FORMAT`] occupies. Unit: bytes.
pub const BYTES_PER_PIXEL: u32 = 4;

/// The largest width or height this driver will accept. Unit: pixels.
///
/// A bound on this driver's own arithmetic before it is a bound on anything
/// else: width times height times [`BYTES_PER_PIXEL`] has to fit a `u32`,
/// because that is what a descriptor's length is and what a transfer command
/// carries. Two thousand and forty-eight squared times four is sixteen
/// mebibytes, which is comfortably inside it and far past anything a component
/// in this tree registers.
///
/// It is not a claim about what a display can show. The device refuses a
/// resource it cannot hold and answers `OUT_OF_MEMORY`, and that refusal reaches
/// the client unchanged — R07 — because a bound this driver invented would be a
/// bound a client could not act on.
pub const DIMENSION_MAX: u32 = 2048;

/// Resources this driver will create before it refuses.
///
/// Eight, and the number matters less than the refusal. This driver sends no
/// `RESOURCE_UNREF` — the module comment says why and what it costs — so every
/// frame it shows leaves a resource behind on the display. Without a bound that
/// is an unbounded leak with a client's name on it; with one it is a quota, and
/// a client that reaches it is told `RESOURCE`/`QUOTA_EXHAUSTED` and can decide
/// what to do. R04: the alternative to refusing is reusing an identifier the
/// display still holds, which is a display showing one client's pixels under
/// another client's name.
///
/// *Reversal:* a surface with an owner. The moment something in this system
/// holds a display resource for longer than one frame, `RESOURCE_UNREF` has a
/// lifetime to hang on and this constant becomes the size of a table rather than
/// the end of the road.
///
/// Unit: resources.
pub const RESOURCES_MAX: u32 = 8;

/// Bytes of the granted region this driver keeps for its command slots.
///
/// One page. It holds the display commands the device reads, the responses it
/// writes, and the scratch [`Driver::provoke_copy`] moves bytes through. None of
/// it is ever a client's pixels — the whole point of the file is that there is no
/// such place. Unit: bytes.
pub const CONTROL_BYTES: u32 = 4096;

/// The least a driver's granted region may be.
///
/// One queue and the control page. `user/virtio-gpu/manifest.toml` declares
/// sixty-four kibibytes, which is this with a great deal of room to spare, and
/// that file says why the slack is the shape's rather than this driver's.
/// Unit: bytes.
pub const GRANT_BYTES: u32 = QUEUE_BYTES + CONTROL_BYTES;

/// Bytes one command slot occupies in the control page. Unit: bytes.
///
/// A hundred and twenty-eight: the largest command in this driver is
/// `TRANSFER_TO_HOST_2D` at fifty-six bytes, the response is twenty-four, and
/// the slot is rounded up so that every slot starts on a boundary a reader can
/// compute in their head. Slack rather than packing, for
/// `user/virtio-net/src/driver.rs`'s reason about its own header slots: a layout
/// whose alignment depends on the fields that happen to be in it is a layout
/// that breaks when a field is added.
const SLOT_BYTES: u32 = 128;

/// Where the device's answer sits inside a command slot. Unit: bytes.
const RESPONSE_AT: u32 = 64;

/// How many command slots the control page holds. Unit: slots.
///
/// Eight, and one [`op::SHOW`] uses six of them — one per command in the
/// sequence, never reused within a sequence. That is deliberate and
/// `sim/src/gpu.rs` states the reason from the model's side: *reusing a slot
/// would let a later request read an earlier one's response*. This driver waits
/// for each answer before sending the next, so reuse would in fact be safe
/// today; a slot per command is what keeps it safe on the day somebody
/// pipelines, and what leaves all six answers readable in memory afterwards when
/// a boot has gone wrong.
const SLOTS: u32 = 8;

/// How many of them one [`op::SHOW`] uses. Unit: slots.
///
/// Six: create, attach, transfer, set scanout, flush, detach. Named rather than
/// counted at the call sites so that the assertion below is about *this
/// driver's sequence* rather than about a number somebody has to keep in their
/// head, and so that adding a command to [`Driver::sequence`] without room for
/// it fails the build rather than overwriting the scratch page.
const COMMANDS_PER_SHOW: u32 = 6;

/// Where [`Driver::provoke_copy`] moves bytes from. Unit: bytes.
const SCRATCH_FROM: u32 = 1024;

/// Where it moves them to. Unit: bytes.
const SCRATCH_TO: u32 = 2048;

/// How much it will move at once. Unit: bytes.
const SCRATCH_BYTES: u32 = 512;

const _: () = assert!(SLOT_BYTES * SLOTS <= SCRATCH_FROM);
const _: () = assert!(SCRATCH_FROM + SCRATCH_BYTES <= SCRATCH_TO);
const _: () = assert!(SCRATCH_TO + SCRATCH_BYTES <= CONTROL_BYTES);
const _: () = assert!(RESPONSE_AT + cmd::HEADER_BYTES <= SLOT_BYTES);
const _: () = assert!(COMMANDS_PER_SHOW <= SLOTS);
// The two spaces that share `error::DEVICE` do not overlap. This crate's own
// codes are single digits and the display's start here, so a client can always
// tell *this driver could not read a register* from *the display refused the
// command*. `crate::Trouble` states the rule and `driver`'s tests check the
// other half of it.
const _: () = assert!(cmd::RESP_OK_NODATA >= cmd::RESP_FIRST);

/// The two descriptors one display command uses.
///
/// Constants and not an allocation, and the same two every time: this driver has
/// one chain outstanding, so the head is fixed and the device gives back the
/// number it was given. That is `user/virtio-blk`'s shape — see the module
/// comment on why a third driver landed back on the first one's.
const CMD_DESC: u16 = 0;
const RESP_DESC: u16 = 1;

/// How many times the used ring is read before a command is called unanswered.
///
/// A count and not a duration, for the reason both other drivers give: what is
/// being waited for is a device, and a duration would need a clock — which RFC
/// 0004 does not offer a component and which would make this boot log a
/// different number on every host. Each turn reads the interrupt-status
/// register, which under emulation is an exit to the emulator and therefore a
/// point at which the device's own work can run.
///
/// Unlike the network driver's receive bound this one is not a policy: the
/// device *owes* an answer to every command in this protocol, so a bound that
/// fires is a broken device rather than a quiet link. That is why it is a
/// constant here and not a number the frame tells the component.
/// Unit: turns.
const COMMAND_LIMIT: u32 = 2_000_000;

/// Registration slots this driver holds per channel.
///
/// Sixteen. A power of two because `f_ring::registry::Table` requires one — the
/// slot index is masked rather than clamped, RFC 0005 — and sixteen because the
/// manifest declares eight clients and a client may hold more than one geometry
/// at a time. A client that runs out is refused `RESOURCE/QUOTA_EXHAUSTED`,
/// which is a peer being told it asked for too much rather than this component
/// deciding how much memory to commit on its behalf.
pub const SETS: usize = 16;

/// The display protocol's own vocabulary: the command header, the six commands
/// this driver sends, and the answers it understands.
///
/// A module of constants and offsets rather than `repr(C)` structs, because
/// everything here is written through [`Region`], which is a bounds-checked
/// volatile accessor and not a reference — there is no struct to borrow. The
/// numbers are the virtio specification's and not this file's.
///
/// **`sim/src/gpu.rs` holds a subset of these and the two must agree.** They do,
/// with one exception that was a defect in the model and is fixed rather than
/// worked around: the model had `OUT_OF_MEMORY` at `0x1202`, which is
/// `INVALID_SCANOUT_ID` in the specification's enumeration. RFC 0054 records it,
/// because a model and a driver that disagree about an error number produce a
/// scenario whose refusal means something else.
pub mod cmd {
    /// Bytes in a command header: type, flags, fence id, context, ring index and
    /// padding. Unit: bytes.
    pub const HEADER_BYTES: u32 = 24;

    /// Where the flags sit in a header. Unit: bytes.
    pub const FLAGS_AT: u32 = 4;
    /// Where the fence identifier sits. Unit: bytes.
    pub const FENCE_AT: u32 = 8;

    /// This command's completion may not be overtaken by a later fenced one.
    pub const FLAG_FENCE: u32 = 1;

    /// Make a two-dimensional resource.
    pub const CREATE_2D: u32 = 0x0101;
    /// Point a scanout at a resource.
    pub const SET_SCANOUT: u32 = 0x0103;
    /// Push a rectangle of a resource to the screen.
    pub const RESOURCE_FLUSH: u32 = 0x0104;
    /// Copy guest memory into a resource.
    pub const TRANSFER_TO_HOST_2D: u32 = 0x0105;
    /// Give a resource guest pages to be made of.
    pub const ATTACH_BACKING: u32 = 0x0106;
    /// Take them away again.
    pub const DETACH_BACKING: u32 = 0x0107;

    /// It worked, and there is nothing to read back.
    pub const RESP_OK_NODATA: u32 = 0x1100;

    /// Nothing has been written here.
    ///
    /// Not a response any device sends, for the same reason `blk`'s status byte
    /// starts at `0xFF`: a driver has to be able to tell *the device refused*
    /// from *the device never answered*. This driver zeroes the response slot
    /// before every command precisely so that this value is reachable.
    pub const RESP_NONE: u32 = 0;

    /// The lowest number the device's own response space uses.
    ///
    /// Everything from here up is a device's word. It is stated as a constant
    /// because [`crate::Trouble`] packs this crate's *own* failures into the same
    /// `DEVICE` error domain with single-digit codes, and the two spaces must
    /// not overlap — a client that could not tell *this driver could not read a
    /// register* from *the display refused the command* could not act on either.
    /// A test asserts the separation.
    pub const RESP_FIRST: u32 = 0x1100;
}

/// What this component did, for the state tree to publish.
///
/// Counts and never durations: the boot log is a fixture that
/// `cargo xtask trace` hashes, and a number that moved with the host would take
/// the fixture with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Entries accepted without a refusal. Unit: entries.
    pub served: u32,
    /// Entries refused. Unit: entries.
    pub refused: u32,
    /// Bytes of pixels the device moved on behalf of clients. Unit: bytes.
    ///
    /// Counted from what the request named, which on this protocol is the only
    /// place the number is: a display command answers a status and never a
    /// length. It is the number [`Counters::copies`] is zero beside, and a zero
    /// beside a zero says nothing.
    pub bytes: u64,
    /// Bytes this component copied on the data path. Unit: bytes.
    ///
    /// **Required to be zero**, and it is a structural property published as a
    /// number rather than a tally of something that happens.
    pub copies: u64,
    /// Backing entries this component pointed past what a registration answered.
    /// Unit: entries.
    ///
    /// Zero on the data path, and moved on purpose by
    /// [`Driver::provoke_escape`], because an isolation proof whose provocation
    /// never ran is the same green as a protection that held.
    pub escaped: u32,
    /// Bytes moved through [`stage`] by [`Driver::provoke_copy`]. Unit: bytes.
    pub provoked: u64,
    /// Completions that carried [`f_abi::cflags::SHORTFALL`]. Unit: completions.
    pub shortfall: u32,
    /// Entries refused `ADMISSION`/`NOT_HELD`: a class the submitting component
    /// was not admitted for. Unit: entries. RFC 0025 bound 2.
    pub unadmitted: u32,
    /// Frames flushed to a scanout. Unit: frames.
    ///
    /// **The one counter in this structure that is not evidence.** A flush that
    /// the device answered `OK` says the display accepted the command; what is
    /// actually on the screen is on the other side of the emulator, and no
    /// number this component can write is a claim about it. `cargo xtask gpu`
    /// captures the screen from outside the machine for exactly that reason, and
    /// RFC 0054 argues why a driver's own report cannot stand in for it.
    pub shown: u32,
    /// Display commands the device answered. Unit: commands.
    pub commands: u32,
    /// Commands the **device** answered with something other than success.
    /// Unit: commands.
    ///
    /// Beside [`Counters::refused`] and never folded into it: one is this
    /// driver's arithmetic and the other is a display's word. It is the counter
    /// the escape provocation moves, because what refuses an address outside a
    /// grant is the remapping unit and what *reports* it is the device failing
    /// to map the backing.
    pub declined: u32,
    /// Resources created and never freed. Unit: resources.
    ///
    /// Published because it is the **cost** of having no `RESOURCE_UNREF`, and
    /// R12 says a concession is written as a cost rather than hidden in a
    /// metric. [`RESOURCES_MAX`] is the bound it runs into.
    pub resources: u32,
    /// Turns of the loop that found nothing on either ring. Unit: turns.
    pub spun: u64,
    /// Operations that failed while the device still held a client's buffer as
    /// the backing of a resource, **and could not be made to let go**.
    /// Unit: operations.
    ///
    /// Required to be zero. It is the counter behind [`Driver::stopped`], and
    /// the module comment argues the whole of why it exists: the ownership
    /// interval on this driver is bounded by a pair of commands rather than by a
    /// chain, so the failure it counts is a detach that did not happen. The
    /// answer to it is a device reset — which blanks the screen — because the
    /// alternative is telling a client its memory is its own while a display is
    /// reading it.
    pub halted: u32,
}

/// What this component is admitted for, and what its channel says about the peer
/// submitting on it.
///
/// Not fields a driver chooses: `crate::routing` argues why a component is told
/// rather than assuming, and a ceiling is the one thing a component must not be
/// able to raise. Identical in shape to the other two drivers' and deliberately
/// not shared with them — the *rule* is `f_abi::deadline::inherit` and lives in
/// `abi/`, which is where a rule three components obey belongs; a struct of three
/// fields that each of them holds is not a rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Admission {
    /// The ceiling this component was admitted for, from its manifest.
    pub mine: Admitted,
    /// The ceiling the channel reports for whoever submits on it.
    pub client: Admitted,
    /// The least time this component needs from arrival to completion.
    /// Unit: nanoseconds.
    pub floor: u64,
}

/// The rectangle one [`op::SHOW`] concerns, checked against the buffer that
/// carries it.
///
/// # Why the geometry is on the entry and not in the manifest
///
/// Because it is the client's. `user/virtio-net`'s frame addresses are the
/// client's for the same reason and that file says it at length: whoever forms
/// the thing chooses what is in it, and a driver with an opinion about what its
/// clients may draw is a driver that has to be reconfigured to draw something
/// else. A display driver that held one geometry as a constant would be a
/// display with one mode, which is not a display.
///
/// # Why it is in `ext` and not in a command of its own
///
/// `Sqe` has two free sixty-four bit words and this protocol has exactly one
/// operation, so a second opcode carrying a geometry would be a round trip and a
/// piece of state this component would have to keep between two entries — and
/// state kept between two entries of one client is the thing a `private` domain
/// is protecting. R03 is satisfied by naming the unit here: both are pixels, and
/// both are refused above [`DIMENSION_MAX`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    /// Unit: pixels.
    pub width: u32,
    /// Unit: pixels.
    pub height: u32,
}

impl Rect {
    /// The geometry an entry names, if it names one.
    ///
    /// # Errors
    ///
    /// `ARGUMENT`/`BAD_ADDRESS` carrying the offending dimension for a zero or
    /// an out-of-range one, and for a length that is not exactly the pixels the
    /// rectangle needs. Exactly, and not *at least*: a buffer longer than the
    /// frame is a client that has misunderstood one of the two numbers, and
    /// serving it would put a rectangle of its choosing on a screen with the
    /// rest of the buffer unexplained.
    pub fn read(entry: &Sqe) -> Result<Self, Refusal> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let (Ok(width), Ok(height)) = (u32::try_from(entry.ext[0]), u32::try_from(entry.ext[1]))
        else {
            return Err((bad, entry.ext[0] | entry.ext[1]));
        };
        if width == 0 || width > DIMENSION_MAX {
            return Err((bad, u64::from(width)));
        }
        if height == 0 || height > DIMENSION_MAX {
            return Err((bad, u64::from(height)));
        }
        // Cannot overflow: `DIMENSION_MAX` squared times four is sixteen
        // mebibytes and this is a `u64` on the way down to a `u32`.
        let needed = u64::from(width) * u64::from(height) * u64::from(BYTES_PER_PIXEL);
        if needed != u64::from(entry.len) {
            return Err((bad, needed));
        }
        Ok(Self { width, height })
    }

    /// How many bytes of pixels it is. Unit: bytes.
    ///
    /// Fits a `u32` by [`Rect::read`]'s bound, and saturates rather than wrapping
    /// so that a `Rect` built by hand in a test cannot produce a smaller number
    /// than it should.
    #[must_use]
    pub const fn bytes(self) -> u32 {
        self.width.saturating_mul(self.height).saturating_mul(BYTES_PER_PIXEL)
    }
}

/// The display driver.
///
/// Holds its transport, one queue, its command page, its registrations and the
/// next resource identifier it will hand the display. In particular it holds no
/// mapping of any client's memory, and there is no field here that could.
///
/// **It is smaller than the network driver's, and the difference is a finding
/// rather than a saving.** RFC 0051 measured the frame's one page of stack
/// against a driver that has to keep a table of posted buffers, and had to
/// shrink that table to four entries to fit. There is no such table here: a
/// display command is a request the device answers, so nothing is outstanding
/// between entries and there is nothing per-buffer to remember. The wall is
/// still there and this driver simply does not reach it, which says the wall
/// belongs to the receive direction rather than to drivers.
pub struct Driver {
    transport: Transport,
    queue: Queue,
    control: Region,
    table: Table<SETS>,
    agreed: Negotiated,
    admission: Admission,
    /// The identifier the next resource will be created with.
    ///
    /// Counting from one, so that zero is never a resource — a display that
    /// treated a zeroed command as naming resource zero would answer a request
    /// nobody made, which is `sim/src/gpu.rs`'s argument for the same choice.
    /// Unit: none — a resource identifier.
    next_resource: u32,
    /// The fence identifier the next fenced command will carry. Unit: none.
    next_fence: u64,
    counters: Counters,
}

impl Driver {
    /// Bring the device up over the windows and the region the supervisor
    /// routed.
    ///
    /// `granted` is the one untyped region `user/virtio-gpu/manifest.toml`
    /// declares, already translated in this component's device domain by the
    /// spawn — which is why the driver does not ask [`Domains`] for it: putting
    /// a component's own declared needs in its domain is the spawn's work, and a
    /// driver that mapped its own queue would be a driver deciding what it was
    /// granted.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a region smaller than [`GRANT_BYTES`], and
    /// anything [`Transport::open`] refuses — including
    /// [`Trouble::NoPlatformAddressing`], which is the refusal that keeps this
    /// driver from putting another component's memory on a screen.
    pub fn start(
        windows: Windows,
        granted: Region,
        agreed: Negotiated,
        admission: Admission,
    ) -> Result<Self, Trouble> {
        if granted.len() < GRANT_BYTES {
            return Err(Trouble::Layout);
        }
        let queue_region = granted.slice(0, QUEUE_BYTES)?;
        let control = granted.slice(QUEUE_BYTES, CONTROL_BYTES)?;

        let transport = Transport::open(windows, QUEUE_SIZE)?;
        let queue = Queue::over(queue_region, transport.size())?;
        // The addresses go in before the queue is enabled, and that ordering is
        // the whole reason `open` and `run` are two calls. A device told to
        // enable a queue whose address registers still hold their reset values
        // is a device pointed at physical address zero.
        transport.queue_at(queue.device_desc()?, queue.device_avail()?, queue.device_used()?)?;
        transport.run()?;

        Ok(Self {
            transport,
            queue,
            control,
            table: Table::new(),
            agreed,
            admission,
            next_resource: 1,
            next_fence: 1,
            counters: Counters::default(),
        })
    }

    /// What this component has done. Unit: see [`Counters`].
    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// What this component is admitted for, and what its channel says about the
    /// peer submitting on it.
    #[must_use]
    pub const fn admission(&self) -> Admission {
        self.admission
    }

    /// Registrations currently live. Unit: buffer sets.
    #[must_use]
    pub fn registrations(&self) -> usize {
        self.table.live()
    }

    /// Record one turn of the caller's loop that found nothing to do.
    ///
    /// A method rather than a field the caller writes, because the counter is
    /// published out of [`Driver::counters`] and a component that could write
    /// one of them directly would be a component with two accounts of what it
    /// did. It is the caller's to call because *idle* is a property of the loop
    /// and not of the driver: this driver is never waiting for anything when it
    /// is not inside [`Driver::execute`].
    ///
    /// It is published for the reason the network driver publishes its own: R12
    /// says a concession is written as a cost. What it costs here is a core spun
    /// between frames, which is what the `irq` need in the manifest would buy
    /// back and what E1-B09 owns.
    pub fn spun(&mut self) {
        self.counters.spun = self.counters.spun.saturating_add(1);
    }

    /// Has a failure left the device holding a client's buffer?
    ///
    /// **Asked once a turn by the caller, and the answer ends its loop.** It is
    /// set at the one place this driver cannot recover: a command failed between
    /// the attach and the detach, and the detach that would have taken the
    /// client's buffer back failed too. The device is reset there — which blanks
    /// the screen, and `crate::transport` says why that is refused everywhere
    /// else — so the refusal the client is then told is safe. What it is not is
    /// something to carry on after: the display has lost every resource it held
    /// and this driver's identifiers name nothing.
    ///
    /// Once it is true every entry is refused rather than served, so a caller
    /// that ignored it drives nothing rather than driving a device in reset.
    #[must_use]
    pub const fn stopped(&self) -> bool {
        self.counters.halted != 0
    }

    /// Decide what one entry is served as here, before it is acted on.
    ///
    /// The answer is `f_abi::deadline::inherit`'s and this adds nothing to it
    /// except the counting and the completion a refusal owes. It is a method on
    /// the driver rather than a free function so that the refusal is *tallied*:
    /// a peer claiming urgency it does not hold is a fact worth a number.
    ///
    /// `now` is passed in rather than read: this crate observes no clock, so the
    /// only caller passes zero and RFC 0025's bound 3 is a constant floor rather
    /// than one measured from arrival.
    ///
    /// # Errors
    ///
    /// The completion to post instead of acting on the entry. Already counted,
    /// so a caller writes it to the ring and does nothing else.
    pub fn admit(&mut self, entry: &Sqe, now: u64) -> Result<Inherited, Cqe> {
        let decided = f_abi::deadline::inherit(
            &Caller::of(entry, self.admission.client),
            Callee { admitted: self.admission.mine, arrival: now, floor: self.admission.floor },
        );
        match decided {
            Ok(order) => Ok(order),
            Err((packed, detail)) => {
                self.counters.refused = self.counters.refused.saturating_add(1);
                if error::unpack(packed).is_some_and(|(domain, code)| {
                    domain == error::ADMISSION && code == error::admission::NOT_HELD
                }) {
                    self.counters.unadmitted = self.counters.unadmitted.saturating_add(1);
                }
                Err(refusal(entry.user_data, packed, detail, now))
            }
        }
    }

    /// Answer one entry.
    ///
    /// Two vocabularies meet here and the dispatch order is RFC 0028's: the two
    /// registration opcodes are handled *instead of* this service's executor
    /// rather than after it, which is why [`Table::execute`] checks the envelope
    /// itself. Everything else is this service's own.
    ///
    /// The signature is `user/virtio-blk`'s — entry in, `Cqe` out — and the
    /// module comment says why a third driver landed back on the first one's
    /// rather than on the second one's `Answered::Later`.
    ///
    /// `now` is passed in rather than read. This crate observes no clock — RFC
    /// 0004 — and a driver that stamped its own completions would be a component
    /// with a second opinion about time.
    pub fn execute<D: Domains>(
        &mut self,
        entry: &Sqe,
        order: Inherited,
        domains: &mut D,
        now: u64,
    ) -> Cqe {
        // The literal is the whole point, and it is the same shape as [`stage`]'s
        // tally-as-an-argument: the address that reaches a backing entry is the
        // one a registration answered, plus a displacement this path passes as a
        // constant zero. There is no field to set and no branch to take.
        self.answer(entry, order, domains, now, 0)
    }

    /// Answer one entry with `beyond` bytes added to the address the
    /// registration resolved to, before it becomes the backing a display reads a
    /// frame out of.
    ///
    /// **A provocation, and it is the third of three in this tree.**
    /// `user/virtio-blk`'s escape has the device *read* memory it was never
    /// granted into a client's buffer; `user/virtio-net`'s has it *write* into
    /// memory nobody granted at a moment nothing chose. This one is a read
    /// again, and what makes it worth a third boot is where the bytes go: a
    /// display puts what it reads on a **screen**, which is outside the machine
    /// and outside every mechanism in this system. An unrefused escape here is
    /// not a corruption to be found in somebody's buffer later, it is a page of
    /// another component's memory rendered to whoever is looking.
    ///
    /// [`Counters::escaped`] counts the backing entries this produced, so a boot
    /// can require that the provocation ran rather than inferring it from a fault
    /// it did not see.
    pub fn provoke_escape<D: Domains>(
        &mut self,
        entry: &Sqe,
        order: Inherited,
        domains: &mut D,
        now: u64,
        beyond: u64,
    ) -> Cqe {
        self.answer(entry, order, domains, now, beyond)
    }

    fn answer<D: Domains>(
        &mut self,
        entry: &Sqe,
        order: Inherited,
        domains: &mut D,
        now: u64,
        beyond: u64,
    ) -> Cqe {
        // Nothing is served once the device has been reset out from under this
        // driver's resource identifiers. R04.
        if self.stopped() {
            self.counters.refused = self.counters.refused.saturating_add(1);
            let mut cqe = refusal(entry.user_data, Trouble::NotResponding.packed(), 0, now);
            self.report_shortfall(&mut cqe, order);
            return cqe;
        }

        if opcode::is_registration(entry.opcode) {
            let mut cqe = self.table.execute(entry, domains, now);
            if cqe.is_error() {
                self.counters.refused = self.counters.refused.saturating_add(1);
            } else {
                self.counters.served = self.counters.served.saturating_add(1);
            }
            self.report_shortfall(&mut cqe, order);
            return cqe;
        }

        let outcome = match entry.opcode {
            op::SHOW => self.perform(entry, now, beyond),
            _ => Err((
                error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
                u64::from(entry.opcode),
            )),
        };

        let mut cqe = match outcome {
            Ok(cqe) => {
                self.counters.served = self.counters.served.saturating_add(1);
                cqe
            }
            Err((packed, detail)) => {
                self.counters.refused = self.counters.refused.saturating_add(1);
                refusal(entry.user_data, packed, detail, now)
            }
        };
        self.report_shortfall(&mut cqe, order);
        cqe
    }

    /// Mark a completion with what the request lost on the way.
    ///
    /// One place rather than one per producer, because *every* answer this
    /// service gives owes the flag and a per-branch version is a branch somebody
    /// adds without it — which is the silent demotion RFC 0025 forecloses,
    /// arrived at by a missing line rather than by a decision. On a refusal as
    /// well as on a success: a request demoted to this service's class and *then*
    /// refused for its geometry was still demoted, and a client re-submitting it
    /// needs to know the class it will be served at next time.
    fn report_shortfall(&mut self, cqe: &mut Cqe, order: Inherited) {
        if order.fell_short() {
            cqe.flags |= cflags::SHORTFALL;
            self.counters.shortfall = self.counters.shortfall.saturating_add(1);
        }
    }

    /// One frame, all the way from a client's registered buffer to a scanout.
    ///
    /// Six commands, in the order the specification requires and the module
    /// comment argues: create, attach, transfer, set scanout, flush, detach.
    ///
    /// The last one is the whole of what this driver knows that neither of the
    /// other two had to: between the attach and the detach the **device** holds
    /// the client's buffer, across four chains that each complete successfully,
    /// and the client may not be told it owns its memory again until the detach
    /// has happened.
    fn perform(&mut self, entry: &Sqe, now: u64, beyond: u64) -> Result<Cqe, Refusal> {
        envelope(entry)?;
        let rect = Rect::read(entry)?;
        if entry.offset != 0 {
            // Nothing on this protocol has an offset. Refused rather than
            // ignored: a field a peer filled in and this side skipped is two
            // peers with different beliefs about what was asked, and a client
            // that meant to show part of a buffer wants a smaller buffer.
            return Err((
                error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
                entry.offset,
            ));
        }
        if self.counters.resources >= RESOURCES_MAX {
            // The quota rather than a wrap. `RESOURCES_MAX` argues it.
            return Err((
                error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                u64::from(RESOURCES_MAX),
            ));
        }

        let name = Name::read(entry, self.agreed.features)?;
        let mut path =
            Registered::bind(self.agreed, &mut self.table).map_err(|packed| (packed, 0))?;
        let reach = path.resolve(name, entry.len)?;

        // The one line where a component's arithmetic decides what a device is
        // pointed at. On the data path `beyond` is a literal zero and this is
        // the address the frame answered; on the escape life it is not, and what
        // refuses the result is the remapping unit rather than anything here.
        let at = reach.address.wrapping_add(beyond);
        if beyond != 0 {
            self.counters.escaped = self.counters.escaped.saturating_add(1);
        }

        let resource = self.next_resource;
        let outcome = self.sequence(resource, at, entry.len, rect);

        // The buffer goes back to the client whatever happened, and after the
        // sequence rather than inside it: `sequence` has already made sure the
        // device is not holding it, either by detaching or by resetting.
        let released = Registered::bind(self.agreed, &mut self.table)
            .map_err(|packed| (packed, 0))?
            .release(name);
        outcome?;
        released?;

        self.counters.bytes = self.counters.bytes.saturating_add(u64::from(entry.len));
        self.counters.shown = self.counters.shown.saturating_add(1);
        Ok(completion(entry.user_data, entry.len as i32, now))
    }

    /// The six commands, and the ownership interval between two of them.
    ///
    /// Split out of [`Driver::perform`] so that the interval is a scope rather
    /// than a comment: everything between the attach and the detach is in this
    /// function, and there is no `?` in it that could return between the two.
    fn sequence(&mut self, resource: u32, at: u64, len: u32, rect: Rect) -> Result<(), Refusal> {
        self.create(resource, rect)?;
        // Counted where the display accepted it and not where it was asked for,
        // because the quota `perform` refuses against is a count of resources
        // the *display* holds.
        self.counters.resources = self.counters.resources.saturating_add(1);
        self.next_resource = self.next_resource.saturating_add(1);

        self.attach(resource, at, len)?;

        // --- the device holds the client's buffer from here ------------------
        //
        // Not a chain and not a descriptor: `ATTACH_BACKING` completed, its
        // descriptors came back, and the display went on holding the address it
        // was given. Every refusal below this line is answered by taking it back
        // first, which is what `undo` is.
        let shown = self
            .transfer(resource, rect)
            .and_then(|()| self.scanout(resource, rect))
            .and_then(|()| self.flush(resource, rect));
        let detached = self.detach(resource);
        // --- and not from here ------------------------------------------------

        if let Err(refused) = detached {
            // The device still holds it and asking again would be asking the
            // same broken thing twice. A reset is the only remaining way to make
            // *the client owns its buffer* true when it is said, and it costs
            // the screen — which `crate::transport` refuses everywhere else and
            // which is the right price here.
            self.counters.halted = self.counters.halted.saturating_add(1);
            self.reset();
            return Err(refused);
        }
        shown
    }

    /// Make a two-dimensional resource of `rect`, fenced.
    ///
    /// Fenced because `sim/src/gpu.rs` fences a creation and does not fence a
    /// transfer, and the two agreeing is worth more than either being right on
    /// its own. What the flag buys *this* driver is nothing: one chain is
    /// outstanding at a time, so there is no later completion for an earlier one
    /// to be overtaken by. It is set because a driver that pipelines needs it
    /// exactly here — a creation that completed after a later creation would
    /// leave the driver unable to say which identifier the display ran out on —
    /// and because a flag added later is a flag added by somebody who has to
    /// rediscover the argument.
    fn create(&mut self, resource: u32, rect: Rect) -> Result<(), Refusal> {
        let slot = slot(0)?;
        self.header(slot, cmd::CREATE_2D, cmd::FLAG_FENCE)?;
        self.put32(slot + 24, resource)?;
        self.put32(slot + 28, FORMAT)?;
        self.put32(slot + 32, rect.width)?;
        self.put32(slot + 36, rect.height)?;
        self.round_trip(slot, 40)
    }

    /// Give the resource the client's buffer to be made of.
    ///
    /// One memory entry, because a registration answers one contiguous [`Reach`]
    /// — the frame's domain gives a set one address — so a scatter list of one is
    /// what this driver has to say. A client whose pixels were not contiguous in
    /// the device's address space would need more entries, and the shape of that
    /// is a loop over a `Reach` per buffer rather than anything new.
    fn attach(&mut self, resource: u32, at: u64, len: u32) -> Result<(), Refusal> {
        let slot = slot(1)?;
        self.header(slot, cmd::ATTACH_BACKING, 0)?;
        self.put32(slot + 24, resource)?;
        self.put32(slot + 28, 1)?;
        self.put64(slot + 32, at)?;
        self.put32(slot + 40, len)?;
        self.put32(slot + 44, 0)?;
        self.round_trip(slot, 48)
    }

    /// Copy the pixels out of the client's buffer into the resource.
    ///
    /// **This is where the zero-copy claim is actually cashed.** The copy is the
    /// device's, from memory the frame translated into its domain, into a
    /// resource on the host side of the emulator. This component names the two
    /// ends and touches neither.
    fn transfer(&mut self, resource: u32, rect: Rect) -> Result<(), Refusal> {
        let slot = slot(2)?;
        self.header(slot, cmd::TRANSFER_TO_HOST_2D, 0)?;
        self.rect(slot + 24, rect)?;
        // The offset into the *backing*, which is zero because this driver shows
        // whole frames from the start of a buffer. `Driver::rect` says why the
        // rectangle is anchored at the origin and what a partial update would
        // need.
        self.put64(slot + 40, 0)?;
        self.put32(slot + 48, resource)?;
        self.put32(slot + 52, 0)?;
        self.round_trip(slot, 56)
    }

    /// Point scanout zero at the resource.
    fn scanout(&mut self, resource: u32, rect: Rect) -> Result<(), Refusal> {
        let slot = slot(3)?;
        self.header(slot, cmd::SET_SCANOUT, 0)?;
        self.rect(slot + 24, rect)?;
        self.put32(slot + 40, 0)?;
        self.put32(slot + 44, resource)?;
        self.round_trip(slot, 48)
    }

    /// Push the rectangle to the screen, fenced.
    fn flush(&mut self, resource: u32, rect: Rect) -> Result<(), Refusal> {
        let slot = slot(4)?;
        self.header(slot, cmd::RESOURCE_FLUSH, cmd::FLAG_FENCE)?;
        self.rect(slot + 24, rect)?;
        self.put32(slot + 40, resource)?;
        self.put32(slot + 44, 0)?;
        self.round_trip(slot, 48)
    }

    /// Take the client's buffer back off the resource.
    ///
    /// The command the task's list of five does not name and this driver cannot
    /// do without. What survives it is the resource, which now holds the pixels
    /// on the host's side of the emulator and goes on being scanned out — so the
    /// picture stays and the client's memory is its own again, which is the pair
    /// of facts `TRANSFER_TO_HOST_2D` exists to make possible.
    fn detach(&mut self, resource: u32) -> Result<(), Refusal> {
        let slot = slot(5)?;
        self.header(slot, cmd::DETACH_BACKING, 0)?;
        self.put32(slot + 24, resource)?;
        self.put32(slot + 28, 0)?;
        self.round_trip(slot, 32)
    }

    /// Write a command header and clear the answer beneath it.
    ///
    /// Clearing is not tidiness: [`cmd::RESP_NONE`] is not a response any device
    /// sends, so a slot this driver zeroed and the device did not touch reads as
    /// *never answered* rather than as some earlier command's success.
    fn header(&mut self, slot: u32, kind: u32, flags: u32) -> Result<(), Refusal> {
        let fence = self.next_fence;
        self.next_fence = self.next_fence.saturating_add(1);
        self.put32(slot, kind)?;
        self.put32(slot + cmd::FLAGS_AT, flags)?;
        self.put64(slot + cmd::FENCE_AT, fence)?;
        self.put32(slot + 16, 0)?;
        self.put32(slot + 20, 0)?;
        self.put32(slot + RESPONSE_AT, cmd::RESP_NONE)?;
        Ok(())
    }

    /// Write the four fields of a rectangle at `at`, always anchored at the
    /// origin.
    ///
    /// The origin because this driver shows one whole frame and never a part of
    /// one: a rectangle with an offset is a partial update, which is what a real
    /// display driver spends its life doing and which needs a client that knows
    /// what it changed. Refusing to express it is cheaper than expressing it
    /// wrongly, and the cost is one full-frame transfer per show.
    fn rect(&mut self, at: u32, rect: Rect) -> Result<(), Refusal> {
        self.put32(at, 0)?;
        self.put32(at + 4, 0)?;
        self.put32(at + 8, rect.width)?;
        self.put32(at + 12, rect.height)?;
        Ok(())
    }

    fn put32(&self, at: u32, value: u32) -> Result<(), Refusal> {
        self.control.put32(at, value).map_err(|packed| (packed, 0))
    }

    fn put64(&self, at: u32, value: u64) -> Result<(), Refusal> {
        self.control.put64(at, value).map_err(|packed| (packed, 0))
    }

    /// Offer one command, ring the doorbell, wait for the answer, and read it.
    ///
    /// Two descriptors: the command the device reads and the response it writes.
    /// A display command carries its arguments inside the command structure, so
    /// there is no third descriptor for a payload — which is the structural
    /// reason the pixels are not in the chain and the address of them is.
    fn round_trip(&mut self, slot: u32, len: u32) -> Result<(), Refusal> {
        let command_at = self.control.device_at(slot).map_err(|packed| (packed, 0))?;
        let response_at =
            self.control.device_at(slot + RESPONSE_AT).map_err(|packed| (packed, 0))?;

        self.queue
            .describe(CMD_DESC, command_at, len, DESC_NEXT, RESP_DESC)
            .map_err(|why| (why.packed(), 0))?;
        self.queue
            .describe(RESP_DESC, response_at, cmd::HEADER_BYTES, DESC_WRITE, 0)
            .map_err(|why| (why.packed(), 0))?;
        self.queue.offer(CMD_DESC).map_err(|why| (why.packed(), 0))?;
        self.transport.kick().map_err(|why| (why.packed(), 0))?;

        let mut left = COMMAND_LIMIT;
        let finished = loop {
            if let Some(done) = self.queue.harvest().map_err(|why| (why.packed(), 0))? {
                break done;
            }
            if left == 0 {
                // A device that never answered a command it owes an answer to.
                // Its own code, because *never answered* and *answered about a
                // chain that does not exist* are different failures and a client
                // told the same thing for both cannot retry one and give up on
                // the other. R07.
                return Err((Trouble::NotAnswered.packed(), u64::from(COMMAND_LIMIT)));
            }
            left -= 1;
            // Reads a register, which is an exit to the emulator. See
            // `COMMAND_LIMIT`.
            let _ = self.transport.poke().map_err(|why| (why.packed(), 0))?;
        };

        // The device's word about which chain it finished, checked against the
        // one chain this queue has out. Cheap here, and checked anyway because a
        // rule applied on one driver's queue and not on another's is a rule
        // somebody will find not applied.
        if finished.head != CMD_DESC {
            return Err((Trouble::Device.packed(), u64::from(finished.head)));
        }
        // A used length shorter than the response header is a device that has
        // not answered, and reading the slot anyway would read whatever this
        // driver put there. R04 at a field a device wrote.
        if finished.written < cmd::HEADER_BYTES {
            return Err((Trouble::ShortUsed.packed(), u64::from(finished.written)));
        }
        self.counters.commands = self.counters.commands.saturating_add(1);

        let answer = self.control.get32(slot + RESPONSE_AT).map_err(|packed| (packed, 0))?;
        if answer != cmd::RESP_OK_NODATA {
            self.counters.declined = self.counters.declined.saturating_add(1);
            // The display's own number, passed through unchanged, which is what
            // `sim/src/gpu.rs` does from the model's side and is R07: a refusal
            // this driver invented a code for is a refusal a client cannot act
            // on. The `DEVICE` domain is shared with this crate's own failures
            // and the two spaces do not overlap — `cmd::RESP_FIRST` says why and
            // a test asserts it.
            let code = u16::try_from(answer).unwrap_or(u16::MAX);
            return Err((error::pack(error::DEVICE, code), u64::from(answer)));
        }
        Ok(())
    }

    /// Put the device back in reset.
    ///
    /// **The one caller is the halt path** and `crate::transport`'s module
    /// comment argues why there is no other: a reset destroys every resource the
    /// display holds and replaces the scanout with nothing, so a display driver
    /// that reset itself on an ordinary ending would throw away the one thing it
    /// was asked to produce. What makes it right here is that the alternative is
    /// worse — a client told its buffer is its own again while a display
    /// controller still holds a mapping it may read on its next refresh.
    ///
    /// It reaches the transport through [`Transport::open`]'s own first act
    /// rather than through a `stop` method, because a `stop` method is a thing
    /// somebody calls in a teardown.
    fn reset(&mut self) {
        // Ignored, and deliberately: this is the path where something has
        // already failed, and a reset that also failed leaves nothing further
        // this component can do. `Counters::halted` is what a boot reads, and
        // `kernel/src/gpu.rs` requires it to be zero.
        let _ = self.transport.reset();
    }

    /// Move [`SCRATCH_BYTES`] bytes inside this component's own control page,
    /// counting them.
    ///
    /// **Not part of the data path, and it exists so that the zero on the data
    /// path is a measurement.** The same argument `kernel/src/mem.rs` makes with
    /// `provoke_remote`: a counter nothing in a boot can move is
    /// indistinguishable from a counter that does not work, so the boot moves one
    /// on purpose and publishes it beside the one that must stay at zero.
    ///
    /// It touches the control page, which holds display commands and has never
    /// held a client's pixels — there is no code in this crate that could put
    /// them there.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a control page too short, which
    /// [`Driver::start`] has already made unreachable.
    pub fn provoke_copy(&mut self) -> Result<(), Trouble> {
        stage(&self.control, SCRATCH_FROM, SCRATCH_TO, SCRATCH_BYTES, &mut self.counters.provoked)
    }
}

/// Move `len` bytes from `from` to `to` inside one region, adding them to
/// `tally`.
///
/// **The only function in this crate that moves bytes**, and the tally is an
/// argument rather than a field so that *which* counter moved says which caller
/// ran. [`Counters::copies`] is the data path's and no caller on the data path
/// passes it; [`Counters::provoked`] is the boot's own self-check's. A reader who
/// wants to disagree with *zero copies on the data path* should start by
/// searching this crate for calls to this function, which is a search with one
/// result — and `cargo xtask lint-datapath` runs that search on every `lint`.
///
/// Byte at a time rather than through a slice, and that is not a performance
/// statement: a [`Region`] hands out no slice at all, for the reason
/// `f_ring::device` gives — a slice asserts exclusive access to memory something
/// else may be writing, and on this driver's control page the something else is
/// a display controller.
///
/// # Errors
///
/// [`Trouble::Register`] for a range outside the region.
/// Where command `which` of a sequence lives in the control page.
///
/// A free function rather than a method because it is arithmetic on one number
/// that is not the driver's state, and because the only way to test a bound is
/// to be able to call the thing that enforces it. `head_for` in
/// `user/virtio-net/src/driver.rs` is a free function for the same reason and
/// records the review finding behind it: a test that recomputes a formula in its
/// own body asserts its own arithmetic against itself.
///
/// # Errors
///
/// `ARGUMENT`/`BAD_ADDRESS` for a slot past the page, which is this driver's own
/// arithmetic gone wrong and is refused rather than wrapped into a slot
/// somebody else is using — which on this page is the scratch the copy
/// self-check moves bytes through, so a wrap would make [`Counters::provoked`]
/// a number a display command had scribbled on.
fn slot(which: u32) -> Result<u32, Refusal> {
    if which >= SLOTS {
        return Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), u64::from(which)));
    }
    Ok(which.saturating_mul(SLOT_BYTES))
}

fn stage(region: &Region, from: u32, to: u32, len: u32, tally: &mut u64) -> Result<(), Trouble> {
    let mut moved = 0;
    while moved < len {
        let byte = region.get8(from.saturating_add(moved))?;
        region.put8(to.saturating_add(moved), byte)?;
        moved += 1;
    }
    *tally = tally.saturating_add(u64::from(len));
    Ok(())
}

/// Refuse an entry this service will not read, in the order `f_ring::execute`
/// fixes: the reserved word, then the flags, then the opcode.
///
/// The order is not cosmetic. An entry with a non-zero reserved word is
/// malformed whatever it claims to be, and reporting the opcode first would tell
/// a caller its opcode was wrong when it was not. R04, and R07: each earns its
/// own code because a client that cannot tell which of them happened cannot
/// handle it as ordinary control flow.
///
/// `ext` is **not** refused here, and that is the one line where this service's
/// envelope differs from both other drivers'. On this protocol the two extension
/// words carry the geometry — [`Rect`] says why there — so a check that required
/// them to be zero would refuse every legal entry. They are checked by
/// [`Rect::read`] instead, which is stricter than a zero test: a geometry that
/// does not match the buffer's length is refused with the length it should have
/// had.
fn envelope(entry: &Sqe) -> Result<(), Refusal> {
    if entry._reserved != 0 {
        return Err((
            error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
            u64::from(entry._reserved),
        ));
    }
    let unknown = entry.flags & !flags::KNOWN;
    if unknown != 0 {
        return Err((
            error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
            u64::from(unknown),
        ));
    }
    if !op::known(entry.opcode) {
        return Err((
            error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
            u64::from(entry.opcode),
        ));
    }
    // A field this service does not read, refused rather than skipped. `cap` is
    // the registration path's and never a transfer's.
    if entry.cap != 0 {
        return Err((
            error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
            u64::from(entry.cap),
        ));
    }
    Ok(())
}

/// Build the entry that puts one buffer of a registered set on the scanout.
///
/// Beside the driver rather than in a client, for the reason
/// `f_ring::registry::registration` sits beside the table that answers it: two
/// accounts of where a field goes is one too many, and a client that had to
/// write these by hand would be a client that can get the envelope wrong — which
/// on this protocol means putting the geometry in the wrong extension word and
/// being told its buffer is the wrong length.
#[must_use]
pub fn show(token: u64, width: u32, height: u32) -> Sqe {
    let mut entry = Sqe::ZERO;
    entry.opcode = op::SHOW;
    entry.user_data = token;
    entry.len = Rect { width, height }.bytes();
    entry.ext[0] = u64::from(width);
    entry.ext[1] = u64::from(height);
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A control page at a descriptor's alignment. As `queue`'s fixture, and for
    /// the same reason: an alignment the compiler happened to give is a test
    /// that passes for a reason nobody chose.
    #[repr(align(16))]
    struct Owned([u8; CONTROL_BYTES as usize]);

    impl Owned {
        const fn new() -> Self {
            Self([0; CONTROL_BYTES as usize])
        }

        fn region(&mut self) -> Region {
            Region::at(self.0.as_mut_ptr() as usize as u64, 0x5000_0000, CONTROL_BYTES)
                .expect("an aligned region")
        }
    }

    #[test]
    fn the_only_function_that_moves_bytes_moves_whichever_tally_it_is_given() {
        // The test that makes `copies = 0` worth reading. Both tallies go
        // through one function, so a zero in one of them is a statement about
        // its callers rather than about the counter — and a build where `stage`
        // stopped counting would fail here rather than publishing two zeroes.
        let mut owned = Owned::new();
        let region = owned.region();
        for byte in 0..SCRATCH_BYTES {
            region.put8(SCRATCH_FROM + byte, 0xA5).expect("inside the page");
        }

        let mut copies = 0u64;
        let mut provoked = 0u64;
        stage(&region, SCRATCH_FROM, SCRATCH_TO, SCRATCH_BYTES, &mut provoked).expect("inside");
        assert_eq!(provoked, u64::from(SCRATCH_BYTES));
        assert_eq!(copies, 0, "the tally that was not passed did not move");

        stage(&region, SCRATCH_FROM, SCRATCH_TO, SCRATCH_BYTES, &mut copies).expect("inside");
        assert_eq!(copies, u64::from(SCRATCH_BYTES), "and it moves when it is");

        assert_eq!(region.get8(SCRATCH_TO), Ok(0xA5));
        assert_eq!(region.get8(SCRATCH_TO + SCRATCH_BYTES - 1), Ok(0xA5));
    }

    #[test]
    fn a_zeroed_entry_names_no_operation() {
        // The reason the opcodes start at one. An entry that was memset — a slot
        // off a free list, a peer that zeroed one — must not read as a show of
        // buffer zero.
        assert!(!op::known(0));
        assert_eq!(
            envelope(&Sqe::ZERO),
            Err((error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE), 0))
        );
    }

    #[test]
    fn a_geometry_that_does_not_describe_the_buffer_is_refused() {
        // The check that replaces the block driver's sector arithmetic and the
        // network driver's frame bounds. What is under test is that the two
        // numbers on the entry and the one on the buffer have to agree exactly:
        // a buffer longer than the rectangle is a client that has misunderstood
        // one of them, and showing it anyway would put a rectangle of this
        // driver's choosing on a screen.
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let good = show(1, 16, 16);
        assert_eq!(Rect::read(&good), Ok(Rect { width: 16, height: 16 }));
        assert_eq!(good.len, 16 * 16 * 4);

        let mut zero_width = good;
        zero_width.ext[0] = 0;
        assert_eq!(Rect::read(&zero_width), Err((bad, 0)));

        let mut huge = good;
        huge.ext[1] = u64::from(DIMENSION_MAX) + 1;
        assert_eq!(Rect::read(&huge), Err((bad, u64::from(DIMENSION_MAX) + 1)));

        let mut short = good;
        short.len -= 4;
        assert_eq!(Rect::read(&short), Err((bad, 16 * 16 * 4)));

        let mut long = good;
        long.len += 4;
        assert_eq!(Rect::read(&long), Err((bad, 16 * 16 * 4)), "longer is refused too");
    }

    #[test]
    fn a_display_response_cannot_be_mistaken_for_this_crate_refusing() {
        // Both go into `error::DEVICE`, so the two code spaces must not
        // overlap: a client that could not tell *this driver could not read a
        // register* from *the display refused the command* could not act on
        // either. R07, applied to a domain two different things share.
        let mine = [
            Trouble::NotResponding,
            Trouble::NoPlatformAddressing,
            Trouble::FeaturesRefused,
            Trouble::NoQueue,
            Trouble::ShortUsed,
            Trouble::Device,
            Trouble::NotAnswered,
        ];
        for trouble in mine {
            let (domain, code) = error::unpack(trouble.packed()).expect("a refusal is negative");
            assert_eq!(domain, error::DEVICE);
            assert!(
                u32::from(code) < cmd::RESP_FIRST,
                "this crate's own DEVICE codes must stay below the display's response space"
            );
        }
    }

    #[test]
    fn a_command_slot_past_the_page_is_refused_rather_than_wrapped() {
        // This driver's own arithmetic, checked the way a device's word is:
        // `slot` is the only thing that turns a command's position in a sequence
        // into an offset, and a sequence that grew past the page would otherwise
        // write a command over the scratch the copy self-check moves bytes
        // through — which would make `provoked` a number a display command had
        // scribbled on.
        assert_eq!(slot(0), Ok(0));
        assert_eq!(slot(COMMANDS_PER_SHOW - 1), Ok((COMMANDS_PER_SHOW - 1) * SLOT_BYTES));
        assert_eq!(
            slot(SLOTS),
            Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), u64::from(SLOTS)))
        );
        // And the last slot a sequence can reach still ends before the scratch,
        // which is the property the refusal above is protecting.
        assert!(slot(SLOTS - 1).expect("the last slot") + SLOT_BYTES <= SCRATCH_FROM);
    }

    #[test]
    fn the_entry_builder_and_the_reader_agree() {
        // Two accounts of where the geometry goes, and this is the one place
        // they meet. A builder that put the width in `ext[1]` would produce
        // entries this driver reads transposed, and every rectangle in this
        // tree is square in the fixtures — which is exactly the shape of defect
        // a test written from one side would miss.
        let entry = show(7, 64, 32);
        assert_eq!(entry.ext[0], 64);
        assert_eq!(entry.ext[1], 32);
        assert_eq!(Rect::read(&entry), Ok(Rect { width: 64, height: 32 }));
        assert_eq!(entry.len, 64 * 32 * 4);
    }
}
