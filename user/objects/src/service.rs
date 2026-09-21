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
//! **Nothing, as of `kernel/src/objects.rs`** — and the paragraph that stood here
//! said otherwise, so it is worth saying what it said. It said an [`Sqe`] reaches
//! this function as an argument because nothing maps a channel for this
//! component, and that the loop which would close it is four lines. Both were
//! true and neither is now: `crate::serve` is that loop, it is closer to forty
//! lines than four, and the frame describes the channel in `prepare_server`.
//!
//! What has not changed is the seam, which is the thing worth keeping. This
//! function still takes an entry and answers a completion, and the loop still
//! wraps it —
//!
//! ```text
//! while let Some(entry) = server.pop()? {
//!     let mut payload = [0u8; PAYLOAD_BYTES];
//!     arena.copy_out(entry.offset as usize, &mut payload);
//!     server.post(service.answer(&entry, &payload, &mut landing, now))?;
//! }
//! ```
//!
//! — so not one line of the counting moved when the loop arrived, which is the
//! property this shape was built for: whoever wrote the transport was not also
//! deciding where an application byte is counted.
//!
//! *What is still owed:* a device. `crate::serve` builds its store in the
//! component's own heap, and `intent/0006-state/spec.md` describes this boot over
//! the blk driver's ring. The boundary above the store is crossed; what is under
//! it is modelled, and `claims/0022` names which of its rows come from where.
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
use f_abi::objects::{Entry, PAYLOAD_BYTES, Read as ReadRecord, Request, Write as WriteRecord, op};
use f_abi::store::refusal;
use f_abi::{Cqe, Sqe, error};
use f_blob::device::Device;
use f_blob::extent::Extent;
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
    /// Writes completed, counted at the entry.
    ///
    /// The same discipline [`Served::reads`] keeps, one direction over: it
    /// moves only in [`Service::answer`] and never in
    /// [`crate::write::WritePath`], so a provocation that drove the path
    /// directly cannot be mistaken for a client that submitted.
    /// Unit: count of writes.
    pub writes: u64,
    /// Application bytes a client submitted to be written.
    ///
    /// **`claims/0017`'s denominator, at the boundary that claim defines it
    /// at.** It is the sum of `f_abi::objects::Write::bytes` — what the client
    /// *asked to write*, per RFC 0058, and not what the store then moved
    /// underneath. `intent/0006-state/spec.md` defines an application byte as
    /// one the client submitted on the objects ring; this is that sum and
    /// there is no other in the tree.
    ///
    /// Deliberately not summed with [`Served::delivered_bytes`]. One is what
    /// landed in a caller's buffer and the other is what left it, the two
    /// claims that rest on them are about different directions, and a single
    /// `bytes` field would make both unreadable.
    /// Unit: bytes.
    pub written_bytes: u64,
    /// The store's address for the most recent client write.
    ///
    /// Published so a client can read back what it wrote: the completion
    /// carries eight bytes of it and a read needs thirty-two. Zero before any
    /// write, which is distinguishable from a real address because SHA-256
    /// over any input is not zero.
    /// Unit: bytes of a SHA-256 digest.
    pub written_hash: [u8; 32],
    /// Bytes moved into new pieces while answering client writes.
    ///
    /// **`claims/0017`'s numerator, at the same boundary as its denominator**,
    /// which is the whole of what `E2-B09` was still open for. The sum of
    /// `f_blob::extent::Cost::copied` over the writes this service answered —
    /// it counts every byte the new piece carries, including the ones the
    /// client already had, which is the quantity RFC 0058 says an extent's cost
    /// is honest about.
    ///
    /// Zero on a channel with no subject, where a write establishes an object
    /// rather than editing one and there is no previous chunking to move.
    /// Unit: bytes.
    pub rechunked_bytes: u64,
    /// Bytes fed to `f-hash` to name those pieces.
    ///
    /// The sum of `f_blob::extent::Cost::hashed`. Equal to
    /// [`Served::rechunked_bytes`] for whole-piece rewrites and kept apart
    /// anyway, for the reason `Cost::hashed`'s own comment gives: a sub-piece
    /// scheme would move the two apart and one row would hide it.
    /// Unit: bytes.
    pub rehashed_bytes: u64,
    /// Pieces rewritten while answering client writes.
    ///
    /// `claims/0017`'s `pieces_touched_max` is a maximum over entries rather
    /// than this sum, and this is its denominator. Unit: count of pieces.
    pub pieces_rewritten: u64,
    /// Bytes spent naming the results — the extent's own record, rewritten.
    ///
    /// **Deliberately not summed into [`Served::rechunked_bytes`].** A snapshot
    /// is proportional to the piece count and therefore to the object's size,
    /// so folding it in would turn `2 x EXTENT_BYTES at both object sizes` into
    /// two numbers that differ by the object — which is the property
    /// `claims/0017` exists to state. It is a real cost of RFC 0098's rule that
    /// a completion carries the object's new address, and is recorded under its
    /// own name rather than dropped.
    /// Unit: bytes.
    pub published_bytes: u64,
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

/// What an entry turned out to ask for, once it was believed.
///
/// Two arms and not a boolean, for `believed`'s own stated reason: what
/// follows differs, and differs in a named function. A third arm arrives with
/// a third service and not before — `op::known` is the gate and this is the
/// shape that makes forgetting one a compile error rather than a silence.
enum Wanted {
    /// Content out of the store and into the caller's buffer.
    Read(ReadRecord),
    /// The caller's bytes into the store.
    Write(WriteRecord),
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
    /// The object this service's channel is about, where it has one.
    ///
    /// RFC 0098: *a `WRITE` edits the object its channel is about*. `None` is a
    /// channel with no object, which is what every channel in a boot is — the
    /// component never calls [`Service::about`], because `Extent::write`
    /// allocates a mebibyte and its heap is 128 KiB. See
    /// [`crate::write`]'s module comment, which is where that decision is
    /// argued rather than restated.
    subject: Option<Extent>,
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
                writes: 0,
                written_bytes: 0,
                written_hash: [0; 32],
                rechunked_bytes: 0,
                rehashed_bytes: 0,
                pieces_rewritten: 0,
                published_bytes: 0,
                staged_bytes: 0,
                refused: 0,
            },
            subject: None,
        }
    }

    /// Say what this service's channel is about.
    ///
    /// # Who calls this, and who deliberately does not
    ///
    /// A caller whose heap can hold `f_blob::extent::EXTENT_BYTES` — a
    /// mebibyte — because that is what `Extent::write` allocates per edited
    /// piece, whatever the object's size. `bench/src/bin/rechunk.rs` is that
    /// caller and takes `claims/0017`'s numerator and denominator across one
    /// boundary because of it.
    ///
    /// `serve.rs` does not, and the omission is the decision rather than an
    /// oversight: the place `kernel/src/objects.rs` builds for this component
    /// has a 128 KiB heap, so a boot's channel has no subject and a non-zero
    /// offset on it is refused. RFC 0098.
    pub fn about(&mut self, extent: Extent) {
        self.subject = Some(extent);
    }

    /// The object this channel is about, for a caller reading back what its
    /// writes produced.
    #[must_use]
    pub const fn subject(&self) -> Option<&Extent> {
        self.subject.as_ref()
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
        match wanted {
            Wanted::Read(wanted) => {
                let outcome = self.read(&wanted, landing, set, request.buf_index);
                self.completed(&request, &wanted, outcome, now)
            }
            Wanted::Write(asked) => self.written(&request, &asked, landing, set, now),
        }
    }

    /// Answer one `WRITE`: the client's bytes, out of its own registered
    /// buffer and into the object store.
    ///
    /// **The completion carries the content address in its detail word**, so a
    /// client can ask for what it just wrote without this component holding a
    /// name on its behalf. Eight bytes of a thirty-two byte hash, which is
    /// enough to name it back to a store that has it and deliberately not
    /// enough to be treated as the address itself — RFC 0013 charges a word
    /// per node for the same reason and this is the same arithmetic.
    fn written(
        &mut self,
        request: &Request,
        asked: &WriteRecord,
        landing: &mut Landing<'_>,
        set: SetId,
        now: u64,
    ) -> Cqe {
        let len = asked.bytes as usize;
        // The bytes, resolved through the caller's own registration. A write
        // whose buffer the table will not answer for is refused before the
        // store is touched, so a refused write costs the object nothing.
        let fetched = match landing.fetch(set, request.buf_index, len) {
            Ok(bytes) => bytes,
            Err(code) => return self.refuse(request.user_data, code, 0, now),
        };
        let applied =
            crate::write::over(&mut self.path, self.subject.as_mut()).apply(asked.offset, fetched);
        // Given back before anything else happens, and **before the refusal
        // below can return**. `Table::resolve` marks a buffer lent and
        // `Table::release` is what unmarks it, so a write path that kept the
        // borrow would refuse its own second entry with `BAD_ADDRESS` — which
        // is exactly what the first draft of this did, and the symptom was a
        // boot that wrote once and then refused every write after it.
        if landing.release(set, request.buf_index).is_err() {
            return self.refuse(request.user_data, refusal::ADDRESS, 0, now);
        }
        let written = match applied {
            Ok(written) => written,
            Err(code) => return self.refuse(request.user_data, code, 0, now),
        };
        self.served.writes = self.served.writes.saturating_add(1);
        self.served.written_bytes = self.served.written_bytes.saturating_add(written.bytes);
        self.served.written_hash = written.hash;
        // The numerator, added up where the denominator is added up. `Cost`
        // hands itself back per call and keeps no total, for the reason
        // `blob/src/extent.rs` gives: a counter inside the write path is a
        // number the write path keeps about itself. This is the first place
        // above it where a *client* can be said to have submitted.
        self.served.rechunked_bytes =
            self.served.rechunked_bytes.saturating_add(written.cost.copied);
        self.served.rehashed_bytes = self.served.rehashed_bytes.saturating_add(written.cost.hashed);
        self.served.pieces_rewritten =
            self.served.pieces_rewritten.saturating_add(u64::from(written.cost.pieces));
        self.served.published_bytes =
            self.served.published_bytes.saturating_add(written.published.copied);
        let named = written.hash.first_chunk::<8>().map_or(0, |head| u64::from_le_bytes(*head));
        let Ok(stated) = i32::try_from(asked.bytes) else {
            return self.refuse(request.user_data, refusal::SHORT_BUFFER, 0, now);
        };
        completion(request.user_data, stated, now).with_ext(named)
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
        // A read provocation, and it stays one. `WRITE` has no staged variant
        // because there is no second copy to model on that side: the store
        // copies the client's bytes into the record it builds and says so, so
        // a *provoked* extra copy there would be modelling a cache that does
        // not exist rather than one that works.
        let Wanted::Read(wanted) = wanted else {
            let packed = error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE);
            return self.refuse(request.user_data, packed, u64::from(request.opcode()), now);
        };
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
    ) -> Result<(Request, Wanted), Cqe> {
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
            Entry::Read(wanted) => Ok((request, Wanted::Read(wanted))),
            Entry::Write(asked) => Ok((request, Wanted::Write(asked))),
            // Unreachable: `op::known` admits `READ` and `WRITE`, and the
            // decode already matched the opcode to the body. Refused rather
            // than panicked, for `f_ring::execute`'s stated reason — an opcode
            // is the most peer-controlled field there is, and *cannot happen*
            // is how a panic gets into a drain loop.
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
