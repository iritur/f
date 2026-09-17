// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The answering side: one submission in, one completion out, and the count
//! taken at that boundary rather than one layer under it.
//!
//! # What changed here, stated before anything else
//!
//! `abi::objects::op::known` used to answer `false` for all five opcodes. It
//! answers `true` for [`op::READ`] now, and this module is the body behind that
//! widening. Everything below it — [`crate::read`], [`crate::dma`],
//! [`crate::resident`] — existed before and is unchanged; what did not exist was
//! anything a *client* could submit to.
//!
//! # Why the count had to move, which is the whole point of the module
//!
//! `intent/0006-state/spec.md` defines an application byte as **one a client
//! submitted on the objects ring**. [`crate::read::Counters::content_bytes`] is
//! not that number and never was: it counts what [`crate::read::ReadPath::read`]
//! delivered, and `ReadPath::read` is a function this crate's own test calls
//! directly. A component calling itself is not a client, so every number taken
//! there is a number about a write path with a reader bolted to it — which is
//! exactly what `claims/0017` and `claims/0022` say about themselves, in the
//! word `pending`.
//!
//! [`Served::delivered_bytes`] is the number at the right boundary. It moves in
//! exactly one place, [`Service::answer`], and only for content that landed in
//! the caller's registered buffer because an [`Sqe`] asked for it. A read this
//! crate did to itself does not move it and cannot, because there is no path
//! into it that does not pass an entry.
//!
//! # What is still missing, said in the same breath
//!
//! **A transport.** An [`Sqe`] arrives here as an argument rather than off a
//! mapped channel, because nothing maps one for this component yet: a place's
//! occupant is given a control ring and no data ring
//! (`kernel/src/component.rs`), and the two components that serve a real
//! channel — `user/virtio-blk` and `user/store` — are each stood up by a
//! purpose-written module in the frame. So what this module is is the *service*
//! and not the *loop*, and the seam is deliberate: the day a channel exists the
//! loop is four lines around this function —
//!
//! ```text
//! while let Some(entry) = server.pop()? {
//!     let mut payload = [0u8; PAYLOAD_BYTES];
//!     arena.copy_out(entry.offset as usize, &mut payload);
//!     server.post(service.answer(&entry, &payload, &mut landing, now))?;
//! }
//! ```
//!
//! — and not one line of the counting moves, because the counting is already at
//! the entry. That is the property worth having from this shape: whoever writes
//! the loop is not also deciding where an application byte is counted.
//!
//! *What this does not license:* calling the boundary crossed. `E2-B08`'s exit
//! is a count taken across the objects ring **in a boot**, `TODO.md` says so in
//! those words, and an argument passed between two functions in one address
//! space is not a boot however honestly the entry was encoded.
//! `user/objects/tests/reads.rs` prints both readings side by side for that
//! reason.
//!
//! # The two refusals that look the same and are not
//!
//! An opcode outside the five is refused by [`Request::decode`] —
//! `Refusal::UnknownOpcode`, the decoder saying the number is not in the
//! vocabulary. An opcode *inside* the five that this build does not answer is
//! refused here, by [`Service::answer`], after the decode succeeded. They pack
//! to the same `ARGUMENT`/`UNKNOWN_OPCODE` on the wire, which is right — a
//! client does the same thing with either, and RFC 0010 says the domain is the
//! stable part — and they are two different things to fix, which is why the
//! second is raised here rather than by asking the decoder to pretend the
//! number is undeclared.
//!
//! # Determinism, and the clock this does not read
//!
//! `now` is an argument. This crate observes no clock — RFC 0004, and the
//! determinism lint would refuse a call to one — so the timestamp a completion
//! carries is the caller's, exactly as `f_ring::execute` takes one and
//! `user/virtio-blk` passes a literal zero for the same reason. A service that
//! read a clock here would make every recorded number a function of the host.

use f_abi::buf::SetId;
use f_abi::objects::{Entry, PAYLOAD_BYTES, Read as ReadRecord, Request, op};
use f_abi::store::refusal;
use f_abi::{Cqe, Sqe, error};
use f_blob::device::Device;
use f_ring::{completion, refusal as refused};
use f_zone::device::Zoned;

use crate::dma::Landing;
use crate::read::ReadPath;

/// What this service did, counted where a client can be said to have asked for
/// it.
///
/// Four numbers and not one, for [`crate::read::Counters`]' reason one module
/// down: a zero beside nothing says nothing. [`Served::delivered_bytes`] is the
/// figure the two claims are defined against and the other three are what make
/// it readable — a delivered count with no entry count behind it cannot be told
/// apart from a service nobody submitted to.
///
/// **Three of these are state nodes**, and the names are the manifest's rather
/// than this file's: `reads`, `delivered` and `staged` are ids 2, 3 and 4 of
/// `user/objects/manifest.toml`. That is deliberate, RFC 0013 — the tree is
/// what a reader outside this component sees, so a counter here that had no
/// node there would be a number only this component's own tests can read.
/// Nothing publishes them yet, because publishing is a write into the region
/// the frame mapped for the state tree and this service is not the thing the
/// frame spawned.
///
/// `entries` and `refused` have no node and are not owed one: they are about
/// this service's own health rather than about the datapath, and a node is a
/// permanent id. `notices` — node 5 — belongs to a control ring, which this
/// module does not have and does not pretend to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Served {
    /// Submissions answered, refusals included.
    ///
    /// The denominator that says a run happened at all. Unit: count of entries.
    pub entries: u64,
    /// Reads completed and content verified, counted at the entry.
    ///
    /// Node 2, `reads`, unit `calls`. Deliberately the same word the manifest
    /// uses: a read is something a client asked for, which is why this moves
    /// only in [`Service::answer`] and never in [`crate::read::ReadPath`].
    /// Unit: count of reads.
    pub reads: u64,
    /// Content bytes that landed in the caller's own registered buffer because
    /// an entry asked for them.
    ///
    /// Node 3, `delivered`, unit `bytes`, and **the denominator `claims/0017`
    /// and `claims/0022` are both defined against**.
    ///
    /// What it counts is what *landed*, not what the completion stated, and the
    /// two differ exactly when a client asks for fewer bytes than the record
    /// holds: the device writes whole blocks into the caller's buffers whatever
    /// the ask, so the bytes are in the caller's memory either way. The
    /// manifest's own words for this node are *content bytes landed in a
    /// caller's own registered buffer*, and a service that counted the smaller
    /// figure would be under-reporting the datapath it is being measured on.
    /// [`Service::answer`]'s completion states the ask; this states the
    /// transfer. Unit: bytes.
    pub delivered_bytes: u64,
    /// Bytes that went through any buffer that was **not** the caller's
    /// registered one, while answering an entry.
    ///
    /// Node 4, `staged`, unit `bytes`, and `E2-B08`'s exit as a number: *copies
    /// per read is zero, counted rather than asserted*. It is a delta off
    /// [`crate::read::Counters::staged_bytes`] taken per entry rather than that
    /// counter read at the end, which is what keeps it a count about the ring:
    /// a provocation somebody ran against the read path directly moves that one
    /// and not this one, and the difference between them is *staging nobody
    /// submitted for*.
    ///
    /// [`Service::provoke_staged_answer`] is what makes a zero here evidence,
    /// by moving it from inside an answer. Unit: bytes.
    pub staged_bytes: u64,
    /// Entries answered with a refusal rather than a result.
    ///
    /// Unit: count of entries. Present because R04 is about refusals being
    /// visible, and a service whose refusals were invisible would publish the
    /// same `reads` as one that was never asked anything wrong.
    pub refused: u64,
}

/// The objects service: a read path, and the entries that reach it.
///
/// It owns the [`ReadPath`] rather than borrowing one, because the counts on
/// both sides of this boundary have to be taken against the same path — a
/// service handed a fresh path per entry would publish a `staged` delta against
/// a counter that had just been reset.
pub struct Service<Z: Zoned, I: Device> {
    path: ReadPath<Z, I>,
    served: Served,
}

impl<Z: Zoned, I: Device> Service<Z, I> {
    /// Put a service in front of a mounted read path.
    pub const fn over(path: ReadPath<Z, I>) -> Self {
        Self {
            path,
            served: Served {
                entries: 0,
                reads: 0,
                delivered_bytes: 0,
                staged_bytes: 0,
                refused: 0,
            },
        }
    }

    /// What this service did, at the entry.
    #[must_use]
    pub const fn served(&self) -> &Served {
        &self.served
    }

    /// The path underneath, for a reader comparing the two boundaries.
    #[must_use]
    pub const fn path(&self) -> &ReadPath<Z, I> {
        &self.path
    }

    /// The path underneath, for a caller with business there — a workload
    /// writing the blobs it is about to ask for, a control that corrupts a
    /// byte.
    ///
    /// Not a way in to the read path for a *client*: everything reachable
    /// through this is reachable without an entry, so nothing it does moves
    /// [`Served::delivered_bytes`]. That is the property, not a gap in it.
    pub fn path_mut(&mut self) -> &mut ReadPath<Z, I> {
        &mut self.path
    }

    /// Do the two readings of the staged bytes agree?
    ///
    /// [`crate::read::ReadPath::readings_agree`] at this boundary, and it is
    /// the same question with a third answer in it: the driver's tally, the
    /// device model's tally, and this service's per-entry sum. All three are
    /// equal on a run where every transfer arrived as an entry, and the sum
    /// being *lower* is a read somebody did without submitting for it.
    #[must_use]
    pub fn readings_agree(&self, landing: &Landing<'_>) -> bool {
        self.path.readings_agree(landing)
            && self.served.staged_bytes == self.path.counters().staged_bytes
    }

    /// Answer one submission.
    ///
    /// The data path. Every byte it delivers goes from the device into the
    /// buffer the entry named, and [`Served::staged_bytes`] is what says so —
    /// not a typestate, and not this function's word.
    ///
    /// Never an `Option<Cqe>`, where `f_ring::execute` returns one: [`flags::NO_CQE`]
    /// is outside `abi::objects::FLAGS_ACCEPTED` and is refused at the decode,
    /// so there is no entry this service may answer with silence. That decision
    /// has its own paragraph in `abi/src/objects.rs` and this signature is what
    /// it buys.
    ///
    /// [`flags`]: f_abi::flags
    pub fn answer(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
        landing: &mut Landing<'_>,
        now: u64,
    ) -> Cqe {
        let (request, wanted) = match self.believed(entry, payload, now) {
            Ok(pair) => pair,
            Err(refusal) => return refusal,
        };
        let set = SetId::from_bits(request.buf_set);
        let outcome = self.read(&wanted, landing, set, request.buf_index);
        self.completed(&request, &wanted, outcome, now)
    }

    /// Answer one submission the way a page cache would have: the first block
    /// into memory this component owns, then moved on into the caller's buffer,
    /// then the read done properly on top of it.
    ///
    /// # Why the service has a provocation and not only the path
    ///
    /// Because [`crate::read::ReadPath::provoke_second_copy`] moves the *path's*
    /// tally and this module's zero is a different number. A run that provoked
    /// only the path would publish `staged_bytes = 0` at the entry beside a
    /// non-zero underneath it and call that evidence, when what it has shown is
    /// that the two counters are not the same counter — which was never in
    /// doubt. So the provocation is available at this boundary too, and
    /// `user/objects/tests/reads.rs` requires it to move this number.
    ///
    /// **Two entry points and not a flag**, which is `user/virtio-blk`'s
    /// discipline one crate over: the data path calls [`Service::answer`], the
    /// provocation is this, and a grep for the name finds every caller. The
    /// client is handed the same bytes either way — what is modelled is a cache
    /// that *works*, so the only difference is that the first block moved twice
    /// and both readings say so.
    ///
    /// # Errors
    ///
    /// Nothing, in the sense that a refusal is a completion. An entry this
    /// service cannot believe is refused exactly as [`Service::answer`] refuses
    /// it, before anything is staged.
    pub fn provoke_staged_answer(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
        landing: &mut Landing<'_>,
        now: u64,
    ) -> Cqe {
        let (request, wanted) = match self.believed(entry, payload, now) {
            Ok(pair) => pair,
            Err(refusal) => return refusal,
        };
        let set = SetId::from_bits(request.buf_set);
        let outcome = self.staged_read(&wanted, landing, request.buf_index, set);
        self.completed(&request, &wanted, outcome, now)
    }

    /// Believe an entry, or produce the completion that refuses it.
    ///
    /// Everything both entry points share, and it is only the disbelief: the
    /// envelope through `Request::decode`, then `op::known`, then the one arm
    /// this build has a body for. What follows differs, and differs in a named
    /// function rather than behind a boolean.
    fn believed(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
        now: u64,
    ) -> Result<(Request, ReadRecord), Cqe> {
        self.served.entries = self.served.entries.saturating_add(1);

        let request = match Request::decode(entry, payload) {
            Ok(request) => request,
            Err(why) => return Err(self.refuse(entry.user_data, why.packed(), 0, now)),
        };
        // The second of the two refusals the module comment distinguishes. The
        // decode above said the number is one of the five; this says this build
        // answers it. Four of the five stop here, and the detail word carries
        // the opcode so that a client chasing one is told *which*.
        if !op::known(request.opcode()) {
            let packed = error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE);
            return Err(self.refuse(entry.user_data, packed, u64::from(request.opcode()), now));
        }
        match request.body {
            Entry::Read(wanted) => Ok((request, wanted)),
            // Unreachable: `op::known` admits `READ` alone and the decode
            // already matched the opcode to the body. Refused rather than
            // panicked, for `f_ring::execute`'s stated reason — an opcode is
            // the most peer-controlled field there is, and *cannot happen* is
            // how a panic gets into a drain loop.
            _ => {
                let packed = error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE);
                Err(self.refuse(entry.user_data, packed, u64::from(request.opcode()), now))
            }
        }
    }

    /// The read, and the staged tally taken across it.
    ///
    /// The delta and not the counter, which is what makes this number about the
    /// ring: whatever the path had staged before this entry arrived is not this
    /// entry's, and a service that read the counter at the end would attribute
    /// a test's direct provocation to a client that never submitted.
    fn read(
        &mut self,
        wanted: &ReadRecord,
        landing: &mut Landing<'_>,
        set: SetId,
        first: u32,
    ) -> Result<crate::read::Read, i32> {
        // A zero-byte ask resolves and moves nothing, which is the reading
        // `abi::objects::Read::bytes` states: *how a client asks whether a hash
        // resolves without moving anything*. Answered before the transfer
        // rather than by transferring and discarding, because discarding a
        // transfer is a transfer.
        if wanted.bytes == 0 {
            return self
                .path
                .locate(&wanted.hash)
                .map(|placement| crate::read::Read {
                    content_at: 0,
                    content_bytes: 0,
                    buffers: 0,
                    placement,
                })
                .ok_or(refusal::ADDRESS);
        }
        let before = self.path.counters().staged_bytes;
        let outcome = self.path.read(&wanted.hash, landing, set, first);
        let staged = self.path.counters().staged_bytes.saturating_sub(before);
        self.served.staged_bytes = self.served.staged_bytes.saturating_add(staged);
        outcome
    }

    /// [`Service::provoke_staged_answer`]'s body: stage, then read.
    ///
    /// The staging goes into the same buffer the read is about to fill, so the
    /// caller ends up holding the bytes it would have held and the only
    /// difference is the ones that moved twice. The delta is taken around
    /// **both** steps, which is the whole reason this is not two calls from the
    /// caller: a provocation counted outside the entry is a provocation at the
    /// wrong boundary, which is the mistake this method exists to make
    /// impossible.
    fn staged_read(
        &mut self,
        wanted: &ReadRecord,
        landing: &mut Landing<'_>,
        first: u32,
        set: SetId,
    ) -> Result<crate::read::Read, i32> {
        let before = self.path.counters().staged_bytes;
        let staged_ok = self.path.provoke_second_copy(&wanted.hash, landing, first);
        let outcome = match staged_ok {
            Ok(_) => self.path.read(&wanted.hash, landing, set, first),
            Err(code) => Err(code),
        };
        let staged = self.path.counters().staged_bytes.saturating_sub(before);
        self.served.staged_bytes = self.served.staged_bytes.saturating_add(staged);
        outcome
    }

    /// Turn what the read path answered into the completion the client reaps.
    ///
    /// `Cqe::result` states the **ask** and [`Served::delivered_bytes`] records
    /// the **transfer**; [`Served::delivered_bytes`]' own comment is where the
    /// difference between the two is argued. `Cqe::ext` carries where the
    /// content begins in the run of buffers, because the record's header lands
    /// with it — `crate::read::Read` says why sliding the content down would be
    /// the one copy this whole task denies — so a client that was not told
    /// would have to assume a header width this format is free to change.
    fn completed(
        &mut self,
        request: &Request,
        wanted: &ReadRecord,
        outcome: Result<crate::read::Read, i32>,
        now: u64,
    ) -> Cqe {
        let read = match outcome {
            Ok(read) => read,
            Err(code) => return self.refuse(request.user_data, code, 0, now),
        };

        let landed = read.content_bytes as u64;
        let asked = u64::from(wanted.bytes);
        let stated = if asked < landed { asked } else { landed };
        // A count that cannot be stated in the completion is refused rather
        // than truncated, `f_ring::write_serial`'s rule: `Cqe::result` is an
        // `i32`, and this is where the width of a wire field becomes a bound on
        // what a read may answer. Refused *after* the transfer because the
        // transfer is what discovered the length — the record's header is the
        // only thing that says how long it is.
        let Ok(stated) = i32::try_from(stated) else {
            return self.refuse(request.user_data, refusal::SHORT_BUFFER, landed, now);
        };

        self.served.reads = self.served.reads.saturating_add(1);
        self.served.delivered_bytes = self.served.delivered_bytes.saturating_add(landed);
        completion(request.user_data, stated, now).with_ext(read.content_at as u64)
    }

    /// One refusal, counted.
    ///
    /// Every refusal in this module goes through here, so that
    /// [`Served::refused`] cannot fall behind a path somebody added later.
    fn refuse(&mut self, user_data: u64, packed: i32, detail: u64, now: u64) -> Cqe {
        self.served.refused = self.served.refused.saturating_add(1);
        refused(user_data, packed, detail, now)
    }
}

/// Put a value in [`Cqe::ext`] without restating the other four fields.
///
/// A private extension trait rather than a constructor in `f_ring`, because
/// `f_ring::completion` is the frame's shape and an objects completion is the
/// first one in this tree that has something to say in `ext` on the
/// *non-error* side. Widening the shared constructor would be a change to
/// every caller of it for one caller's benefit.
///
/// *What would reverse this:* a second service wanting the same thing, at which
/// point the argument belongs in `f_ring::completion` and this trait goes.
trait WithExt {
    /// The same completion, carrying `ext`.
    fn with_ext(self, ext: u64) -> Self;
}

impl WithExt for Cqe {
    fn with_ext(self, ext: u64) -> Self {
        Self { ext, ..self }
    }
}
