// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The block server: the entries it answers, and the counter that says the
//! bytes never passed through it.
//!
//! # What a request names, and what it therefore cannot be
//!
//! A request names a **registered buffer set and an index**, never an address
//! and never a payload. `user/virtio-blk/manifest.toml` declares
//! `payload = "registered"` and this file is what makes that declaration a
//! mechanism: [`Registered::resolve`](f_ring::registry::Registered) answers a
//! [`Reach`](f_ring::registry::Reach), a `Reach` is an address and a length and
//! deliberately not a slice, and a `Reach` goes straight into a descriptor. At
//! no point does this component hold a reference to a client's bytes, so at no
//! point can it copy them. The other path is refused rather than absent: an
//! entry carrying an address earns `ARGUMENT/FEATURE_NOT_NEGOTIATED`, because
//! this channel did not agree shared virtual memory and RFC 0024 says a service
//! that resolves through a table refuses one that does not.
//!
//! # The counter, and why there are two of them
//!
//! E1-B02's exit is *zero copies on the data path, verified by counter*, and a
//! counter that says zero because nothing could ever move it is not a
//! verification — it is the same defect `state::node::MEMORY_FORCED` exists to
//! take out of the allocator's remote-allocation number, and the same one
//! `cargo xtask trace` exists to catch one layer down.
//!
//! So there is exactly one function in this crate that moves bytes — [`stage`]
//! — and it takes the tally it moves as an argument. The data path never calls
//! it, so [`Counters::copies`] is zero. [`Driver::provoke_copy`] calls it
//! against the driver's own scratch memory, so [`Counters::provoked`] is not.
//! Two numbers, one mechanism, and the second is what makes the first mean
//! something: a build in which `stage` had been deleted, or had stopped
//! counting, would publish a zero in *both*.
//!
//! That is still only half of it, and the missing half was found in review:
//! *there is exactly one function that moves bytes* is a claim about this
//! file's source, and a build that had grown a second one would publish the
//! same zero while copying every request. `cargo xtask lint-datapath` is what
//! makes it a mechanism — one definition of [`stage`], one call to it, that
//! call inside [`Driver::provoke_copy`], and no shipped line of this crate
//! minting a [`Region`] or a `Window` out of an address it invented. The last
//! clause is the one a reader will not guess at: `Region::at` is a safe
//! `const fn`, so nothing in the language stops a crate that forbids `unsafe`
//! from naming the direct map, building a region over a client's buffer and
//! reading it through the accessor RFC 0033 made safe — with [`stage`]
//! untouched and [`Counters::copies`] still zero. The lint refuses that line;
//! the counter cannot see it.
//!
//! Both are published through the state tree — `kernel/src/state.rs`, RFC
//! 0013 — rather than asserted in a comment, which is the difference the exit
//! criterion is asking for.
//!
//! # One request at a time, and the deadline field that is read and not used
//!
//! [`Driver::execute`] answers one entry before it takes the next.
//! `Sqe::deadline` and `Sqe::class` are carried into the completion untouched
//! and order nothing, because ordering a device queue by the deadline field is
//! E1-B06's and doing it here would be this file deciding a question that task
//! owns. What is true today is written down rather than implied: this driver is
//! first-come-first-served, and its manifest's `features` list is empty for
//! exactly that reason.

use f_abi::buf::{Name, opcode};
use f_abi::{Cqe, Negotiated, Sqe, error, flags};
use f_ring::device::Region;
use f_ring::registry::{Domains, Refusal, Registered, Table, Transport as _};
use f_ring::{completion, refusal};

use crate::Trouble;
use crate::queue::{DESC_NEXT, DESC_WRITE, QUEUE_BYTES, QUEUE_SIZE, Queue};
use crate::transport::{SECTOR_BYTES, Transport, Windows};

/// The opcodes this service answers on.
///
/// Numbered from one and not from zero, and the reason is R04 rather than
/// taste: `f_abi::op::NOP` is zero in the frame's own vocabulary, and an entry
/// that arrived here zeroed — a slot pulled off a free list, a peer that
/// memset an entry — would otherwise be a *read of sector zero into buffer
/// zero*. Zero names nothing here, so a zeroed entry is refused.
///
/// The space is this service's, as `ring-scene-boot` section 05 says: a storage
/// ring and a compositor ring share the envelope and not the words. The two
/// registration opcodes at the top of the byte are the exception RFC 0028
/// argues for, and `f_abi::buf::opcode::is_registration` is what keeps them out
/// of this list.
pub mod op {
    /// Read `len` bytes from `offset` into the named buffer. The device writes.
    pub const READ: u8 = 1;

    /// Write `len` bytes from the named buffer to `offset`. The device reads.
    pub const WRITE: u8 = 2;

    /// Is this an opcode this service implements?
    ///
    /// The negative answer is the one that matters: everything else is refused
    /// with `ARGUMENT/UNKNOWN_OPCODE` rather than being read as the nearest
    /// thing, which is R04 at the one place a client's mistake would otherwise
    /// become a transfer.
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, READ | WRITE)
    }
}

/// Bytes of the granted region this driver keeps for its own request headers.
///
/// One page. It holds a sixteen-byte block request header, the status byte the
/// device writes back, and the scratch [`Driver::provoke_copy`] moves bytes
/// through. None of it is ever a client's data — the whole point of the file is
/// that there is no such place — so a page is generous and stays a page.
/// Unit: bytes.
pub const CONTROL_BYTES: u32 = 4096;

/// The least a driver's granted region may be.
///
/// The queue and the control page. `user/virtio-blk/manifest.toml` declares
/// sixty-four kibibytes, which is this with room to spare, and the difference
/// is deliberate: a manifest sized to exactly what a build needs is a manifest
/// that has to change every time the build does.
/// Unit: bytes.
pub const GRANT_BYTES: u32 = QUEUE_BYTES + CONTROL_BYTES;

/// Where the request header sits in the control page. Unit: bytes.
const HEADER_AT: u32 = 0;

/// Bytes in a block request header: type, priority, sector. Unit: bytes.
const HEADER_BYTES: u32 = 16;

/// Where the status byte the device writes sits. Unit: bytes.
const STATUS_AT: u32 = 16;

/// Where [`Driver::provoke_copy`] moves bytes from. Unit: bytes.
const SCRATCH_FROM: u32 = 64;

/// Where it moves them to. Unit: bytes.
const SCRATCH_TO: u32 = 2048;

/// How much it will move at once. Unit: bytes.
const SCRATCH_BYTES: u32 = 512;

const _: () = assert!(SCRATCH_TO + SCRATCH_BYTES <= CONTROL_BYTES);
const _: () = assert!(SCRATCH_FROM + SCRATCH_BYTES <= SCRATCH_TO);

/// A block read: the device writes the buffer.
const BLK_IN: u32 = 0;

/// A block write: the device reads the buffer.
const BLK_OUT: u32 = 1;

/// The status byte a device writes for a request it completed.
const BLK_OK: u8 = 0;

/// The value the status byte is set to before a request is offered.
///
/// Not a status any device defines, so a status still holding it afterwards
/// means *nothing was written here* rather than *the device reported success* —
/// which is a distinction `dma.rs` had to make the hard way and records: this
/// emulator answers a refused transfer with a successful completion.
const BLK_UNANSWERED: u8 = 0xFF;

/// The three descriptors one request uses.
const DESC_HEADER: u16 = 0;
const DESC_DATA: u16 = 1;
const DESC_STATUS: u16 = 2;

/// How many times the used ring is read before a transfer is called lost.
///
/// A count and not a duration, for the reason `vtd` gives at its own spin
/// bound and `dma.rs` repeats: what is being waited for is a device, and a
/// duration would need a clock — which RFC 0004 does not offer a component and
/// which would make this boot log a different number on every host. Each turn
/// reads the interrupt-status register, which under emulation is an exit to the
/// emulator and therefore a point at which the device's own work can run.
///
/// *Reversal, and it has an owner:* the manifest declares an `irq`, and waiting
/// on it rather than spinning is E1-B09's. This constant goes away in the same
/// change.
const POLL_LIMIT: u32 = 2_000_000;

/// Registration slots this driver holds per channel.
///
/// Sixteen. A power of two because `f_ring::registry::Table` requires one — the
/// slot index is masked rather than clamped, RFC 0005 — and sixteen because the
/// manifest declares eight clients and a client with two geometries wants two
/// sets. A client that runs out is refused `RESOURCE/QUOTA_EXHAUSTED`, which is
/// a peer being told it asked for too much rather than this component deciding
/// how much memory to commit on its behalf.
pub const SETS: usize = 16;

/// What this component did, for the state tree to publish.
///
/// Counts and never durations: the boot log is a fixture that
/// `cargo xtask trace` hashes, and a number that moved with the host would take
/// the fixture with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Entries answered without a refusal. Unit: entries.
    pub served: u32,
    /// Entries refused. Unit: entries.
    pub refused: u32,
    /// Bytes the device transferred on behalf of clients. Unit: bytes.
    ///
    /// Counted from what the *request* named rather than from what the device
    /// reported, because the two are different claims and this one is about
    /// the datapath's shape: it is the number [`Counters::copies`] is zero
    /// beside, and a zero beside a zero says nothing.
    pub bytes: u64,
    /// Bytes this component copied on the data path. Unit: bytes.
    ///
    /// **Required to be zero**, and it is a structural property published as a
    /// number rather than a tally of something that happens: nothing on the
    /// data path can move it, which is the claim. The module comment says what
    /// keeps that true — the only function that moves bytes takes its tally as
    /// an argument, no caller on the data path passes this one, and
    /// `cargo xtask lint-datapath` is what turns *only* and *no* from prose
    /// into a check with a fixture that breaks it.
    pub copies: u64,
    /// Descriptors this component pointed past what a registration answered.
    /// Unit: descriptors.
    ///
    /// Zero on the data path, and for a reason that is worth separating from
    /// the one above: [`Counters::copies`] is zero because there is no code
    /// that could move it, and this is zero because
    /// [`Driver::provoke_escape`] is the only caller that can and the data path
    /// is not it. The boot's `escape` half moves it on purpose, and the run
    /// fails if it did not — an isolation proof whose provocation never ran is
    /// the same green as a protection that held.
    pub escaped: u32,
    /// Bytes moved through the same function by [`Driver::provoke_copy`].
    /// Unit: bytes.
    ///
    /// Not part of the data path and published beside it for exactly the reason
    /// `state::node::MEMORY_FORCED` is published beside `MEMORY_REMOTE`: a
    /// counter nothing can move is not a counter, so the boot moves this one on
    /// purpose and a reader can see that the mechanism behind the zero works.
    pub provoked: u64,
}

/// The block driver.
///
/// Holds its transport, its queue, its control page and its registrations, and
/// nothing else. In particular it holds no mapping of any client's memory, and
/// there is no field here that could.
pub struct Driver {
    transport: Transport,
    queue: Queue,
    control: Region,
    table: Table<SETS>,
    agreed: Negotiated,
    /// Sectors the device says it holds. Unit: sectors of [`SECTOR_BYTES`].
    capacity: u64,
    counters: Counters,
}

impl Driver {
    /// Bring the device up over the windows and the region the supervisor
    /// routed.
    ///
    /// `granted` is the one untyped region `user/virtio-blk/manifest.toml`
    /// declares, already translated in this component's device domain by the
    /// spawn — which is why the driver does not ask
    /// [`Domains`] for it: putting a component's own declared needs in its
    /// domain is the spawn's work, and a driver that mapped its own queue would
    /// be a driver deciding what it was granted.
    ///
    /// The split is this driver's, as the manifest says it is: the queue first,
    /// then the control page.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a region smaller than [`GRANT_BYTES`], and
    /// anything [`Transport::open`] refuses — including
    /// [`Trouble::NoPlatformAddressing`], which is the refusal that keeps this
    /// driver from running with no isolation at all.
    pub fn start(windows: Windows, granted: Region, agreed: Negotiated) -> Result<Self, Trouble> {
        if granted.len() < GRANT_BYTES {
            return Err(Trouble::Layout);
        }
        let queue_region = granted.slice(0, QUEUE_BYTES)?;
        let control = granted.slice(QUEUE_BYTES, CONTROL_BYTES)?;

        let transport = Transport::open(windows, QUEUE_SIZE)?;
        let queue = Queue::over(queue_region, transport.size())?;
        // The addresses go in before the queue is enabled, and that ordering is
        // the whole reason `open` and `run` are two calls: a device told to
        // enable a queue whose address registers still hold their reset values
        // is a device pointed at physical address zero.
        transport.queue_at(queue.device_desc()?, queue.device_avail()?, queue.device_used()?)?;
        transport.run()?;
        let capacity = transport.capacity()?;

        Ok(Self {
            transport,
            queue,
            control,
            table: Table::new(),
            agreed,
            capacity,
            counters: Counters::default(),
        })
    }

    /// What this component has done. Unit: see [`Counters`].
    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// Sectors the device says it holds. Unit: sectors of [`SECTOR_BYTES`].
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Registrations currently live. Unit: buffer sets.
    #[must_use]
    pub fn registrations(&self) -> usize {
        self.table.live()
    }

    /// Put the device back in reset.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn stop(&self) -> Result<(), Trouble> {
        self.transport.stop()
    }

    /// Answer one entry.
    ///
    /// Two vocabularies meet here and the dispatch order is RFC 0028's: the two
    /// registration opcodes are handled *instead of* this service's executor
    /// rather than after it, which is why [`Table::execute`] checks the
    /// envelope itself. Everything else is this service's own.
    ///
    /// `now` is passed in rather than read. This crate observes no clock —
    /// RFC 0004 — and a driver that stamped its own completions would be a
    /// component with a second opinion about time.
    pub fn execute<D: Domains>(&mut self, entry: &Sqe, domains: &mut D, now: u64) -> Cqe {
        // The literal is the whole point, and it is the same shape as
        // [`stage`]'s tally-as-an-argument: the address that reaches a
        // descriptor is the one a registration answered, plus a displacement
        // this path passes as a constant zero. There is no field to set and no
        // branch to take.
        self.answer(entry, domains, now, 0)
    }

    /// Answer one entry with `beyond` bytes added to the address the
    /// registration resolved to, before it becomes a descriptor.
    ///
    /// **A provocation, and the one E1-B01's exit could not perform.** That
    /// exit's last clause is *a driver component provably cannot address memory
    /// outside its grant*, and there are two different things a boot can show
    /// about it. One is that a translation withdrawn under a live registration
    /// makes an in-flight transfer fault instead of landing — RFC 0024's
    /// reclaim, which is the frame's property and which `blk=outside` shows.
    /// The other is this one: the component's *own arithmetic* produces an
    /// address it was never granted, writes it into a descriptor, and rings the
    /// doorbell. A `Reach` is an address and a length, addresses are integers,
    /// and nothing in this crate's types stops a driver from adding to one —
    /// which is exactly why the answer has to be the remapping unit and not a
    /// type. This is the call that asks it.
    ///
    /// The sentence that used to close this paragraph — *the code doing the
    /// arithmetic executes in the frame today, because nothing schedules a
    /// component* — is the one RFC 0047 removed. It runs at ring 3, in an
    /// address space where the only memory it can reach is what its manifest
    /// declared, which is what makes the descriptor below an address the
    /// component was never granted rather than one the frame happened to have
    /// mapped anyway.
    ///
    /// [`Counters::escaped`] counts the descriptors this produced, so a boot
    /// can require that the provocation ran rather than inferring it from a
    /// fault it did not see.
    pub fn provoke_escape<D: Domains>(
        &mut self,
        entry: &Sqe,
        domains: &mut D,
        now: u64,
        beyond: u64,
    ) -> Cqe {
        self.answer(entry, domains, now, beyond)
    }

    fn answer<D: Domains>(&mut self, entry: &Sqe, domains: &mut D, now: u64, beyond: u64) -> Cqe {
        if opcode::is_registration(entry.opcode) {
            let cqe = self.table.execute(entry, domains, now);
            if cqe.is_error() {
                self.counters.refused += 1;
            } else {
                self.counters.served += 1;
            }
            return cqe;
        }
        match self.transfer(entry, now, beyond) {
            Ok(cqe) => {
                self.counters.served += 1;
                cqe
            }
            Err((packed, detail)) => {
                self.counters.refused += 1;
                refusal(entry.user_data, packed, detail, now)
            }
        }
    }

    /// One read or one write, all the way to the device and back.
    ///
    /// `beyond` is added to the address the registration answered, and every
    /// caller on the data path passes zero. See [`Driver::provoke_escape`].
    fn transfer(&mut self, entry: &Sqe, now: u64, beyond: u64) -> Result<Cqe, Refusal> {
        envelope(entry)?;
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);

        // The grain, both ends. Refused rather than rounded: a length rounded up
        // to a sector is a device given a buffer's neighbour, which is the
        // failure `iommu::Grant::pages` refuses for the same reason one layer
        // down.
        if entry.len == 0 || !entry.len.is_multiple_of(SECTOR_BYTES) {
            return Err((bad, u64::from(entry.len)));
        }
        if !entry.offset.is_multiple_of(u64::from(SECTOR_BYTES)) {
            return Err((bad, entry.offset));
        }
        let sector = entry.offset / u64::from(SECTOR_BYTES);
        let sectors = u64::from(entry.len / SECTOR_BYTES);
        // The bound the fourth routed window buys. A request past the end of the
        // disk is refused here rather than discovered as a device error, which
        // is the difference between a client that can act on the answer and one
        // that has to guess.
        match sector.checked_add(sectors) {
            Some(end) if end <= self.capacity => {}
            _ => return Err((bad, entry.offset)),
        }

        // A block *read* is a transfer the device performs *into* the client's
        // buffer, so the direction of the descriptor is the opposite of the
        // word in the opcode. Named for who writes rather than for who asked,
        // because the descriptor flag is about the former and a variable named
        // `read` would be true where `DESC_WRITE` is set.
        let device_writes = entry.opcode == op::READ;
        let name = Name::read(entry, self.agreed.features)?;
        // The registration table, and the only place a name becomes an address.
        // `Registered` refuses `Name::Virtual` on this channel, which is what
        // makes *the bytes never reach this component* structural rather than
        // careful: there is no branch here that takes a client's address.
        let mut path = Registered::bind(self.agreed, &mut self.table).map_err(|packed| {
            // Unreachable today — the registered path requires no feature — and
            // handled rather than unwrapped, because `REQUIRES` is a constant
            // somebody may change and an `expect` would turn that into a dead
            // component instead of a refused entry.
            (packed, 0)
        })?;
        let reach = path.resolve(name, entry.len)?;

        // The one line where a component's arithmetic decides what a device is
        // pointed at. On the data path `beyond` is a literal zero and this is
        // the address the frame answered; on the `escape` half it is not, and
        // what refuses the result is the remapping unit rather than anything
        // here — which is the property, stated where it is actually decided.
        let at = reach.address.wrapping_add(beyond);
        if beyond != 0 {
            self.counters.escaped = self.counters.escaped.saturating_add(1);
        }
        let outcome = self.round_trip(at, entry.len, sector, device_writes);
        // The buffer goes back to the client whatever happened, and before the
        // outcome is looked at. A refusal that left a buffer lent is a client
        // that can never submit that index again — the service's own bookkeeping
        // turning one failed request into a permanently lost buffer.
        let released = Registered::bind(self.agreed, &mut self.table)
            .map_err(|packed| (packed, 0))?
            .release(name);
        let device = outcome?;
        released?;

        self.counters.bytes = self.counters.bytes.saturating_add(u64::from(entry.len));
        // `entry.len` and not what the device reported, and the completion says
        // so: a completion is evidence the device finished and never evidence
        // that bytes moved. `dma.rs` records this emulator answering a refused
        // transfer with a successful status, and a driver that reported the
        // device's opinion as fact would be wrong here, today.
        let mut answer = completion(entry.user_data, entry.len as i32, now);
        answer.ext = u64::from(device);
        Ok(answer)
    }

    /// Build the chain, offer it, ring the doorbell, and wait for the used ring.
    ///
    /// Answers how many bytes the device said it wrote. Unit: bytes.
    fn round_trip(
        &mut self,
        at: u64,
        len: u32,
        sector: u64,
        device_writes: bool,
    ) -> Result<u32, Refusal> {
        let device = error::pack(error::DEVICE, 0);

        // The request header, in the driver's own control page. Sixteen bytes
        // the device reads: what kind, a priority nothing uses, and the sector.
        let kind = if device_writes { BLK_IN } else { BLK_OUT };
        self.control.put32(HEADER_AT, kind).map_err(|packed| (packed, 0))?;
        self.control.put32(HEADER_AT + 4, 0).map_err(|packed| (packed, 0))?;
        self.control.put64(HEADER_AT + 8, sector).map_err(|packed| (packed, 0))?;
        self.control.put8(STATUS_AT, BLK_UNANSWERED).map_err(|packed| (packed, 0))?;

        let header_at = self.control.device_at(HEADER_AT).map_err(|packed| (packed, 0))?;
        let status_at = self.control.device_at(STATUS_AT).map_err(|packed| (packed, 0))?;

        // Three descriptors: the header, which the device always reads; the
        // data, whose direction is the request's; and the status byte, which the
        // device always writes. `at` is the client's buffer *in the device's
        // address space* and is the only address in this chain that did not come
        // from this component's own grant — it came from the frame, in answer to
        // a registration, and if the frame has since taken it back the device
        // faults here rather than writing into memory somebody else now owns.
        let data_flags = DESC_NEXT | if device_writes { DESC_WRITE } else { 0 };
        self.queue
            .describe(DESC_HEADER, header_at, HEADER_BYTES, DESC_NEXT, DESC_DATA)
            .map_err(|why| (why.packed(), 0))?;
        self.queue
            .describe(DESC_DATA, at, len, data_flags, DESC_STATUS)
            .map_err(|why| (why.packed(), 0))?;
        self.queue
            .describe(DESC_STATUS, status_at, 1, DESC_WRITE, 0)
            .map_err(|why| (why.packed(), 0))?;

        self.queue.offer(DESC_HEADER).map_err(|why| (why.packed(), 0))?;
        self.transport.kick().map_err(|why| (why.packed(), 0))?;

        let mut left = POLL_LIMIT;
        let written = loop {
            if let Some(written) = self.queue.harvest().map_err(|why| (why.packed(), 0))? {
                break written;
            }
            if left == 0 {
                // A device that never answered. `DEVICE` is the domain RFC 0010
                // puts hardware failures in, and the detail is the poll bound
                // rather than a status — there is no status, which is the point.
                return Err((device, u64::from(POLL_LIMIT)));
            }
            left -= 1;
            // Reads a register, which is an exit to the emulator. See
            // `POLL_LIMIT`.
            let _ = self.transport.poke().map_err(|why| (why.packed(), 0))?;
        };

        let status = self.control.get8(STATUS_AT).map_err(|packed| (packed, 0))?;
        if status != BLK_OK {
            return Err((device, u64::from(status)));
        }
        Ok(written)
    }

    /// Move [`SCRATCH_BYTES`] bytes inside this component's own control page,
    /// counting them.
    ///
    /// **Not part of the data path, and it exists so that the zero on the data
    /// path is a measurement.** The same argument `kernel/src/mem.rs` makes
    /// with `provoke_remote`: a counter nothing in a boot can move is
    /// indistinguishable from a counter that does not work, so the boot moves
    /// one on purpose and publishes it beside the one that must stay at zero.
    ///
    /// It touches the control page, which holds request headers and a status
    /// byte and has never held a client's bytes — there is no code in this
    /// crate that could put them there.
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
/// passes it; [`Counters::provoked`] is the boot's own self-check's. A reader
/// who wants to disagree with *zero copies on the data path* should start by
/// searching this crate for calls to this function, which is a search with two
/// results — and `cargo xtask lint-datapath` runs that search on every `lint`
/// so that the answer stays two rather than being re-established by whoever
/// next reads the file.
///
/// Byte at a time rather than through a slice, and that is not a performance
/// statement: a [`Region`] hands out no slice at all, for the reason
/// `f_ring::device` gives — a slice asserts exclusive access to memory
/// something else may be writing.
///
/// # Errors
///
/// [`Trouble::Register`] for a range outside the region.
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
/// malformed whatever it claims to be, and reporting the opcode first would
/// tell a caller its opcode was wrong when it was not. R04, and R07: each earns
/// its own code because a client that cannot tell which of them happened cannot
/// handle it as ordinary control flow.
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
    // Fields this service does not read, refused rather than skipped: a field a
    // peer filled in and this side ignored is two peers with different beliefs
    // about what was asked. `cap` is the registration path's and never a
    // transfer's, and `ext` is nobody's yet.
    let unread = u64::from(entry.cap) | entry.ext[0] | entry.ext[1];
    if unread != 0 {
        return Err((error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO), unread));
    }
    Ok(())
}

/// Build the entry that reads `len` bytes from `offset` into one buffer of a
/// registered set.
///
/// Beside the driver rather than in a client, for the reason
/// `f_ring::registry::registration` sits beside the table that answers it: two
/// accounts of where a field goes is one too many, and a client that had to
/// write these by hand would be a client that can get the envelope wrong.
#[must_use]
pub fn read(token: u64, offset: u64, len: u32) -> Sqe {
    let mut entry = Sqe::ZERO;
    entry.opcode = op::READ;
    entry.user_data = token;
    entry.offset = offset;
    entry.len = len;
    entry
}

/// Build the entry that writes `len` bytes from one buffer of a registered set
/// to `offset`.
#[must_use]
pub fn write(token: u64, offset: u64, len: u32) -> Sqe {
    let mut entry = Sqe::ZERO;
    entry.opcode = op::WRITE;
    entry.user_data = token;
    entry.offset = offset;
    entry.len = len;
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A control page at a descriptor's alignment. As `queue`'s fixture, and
    /// for the same reason: an alignment the compiler happened to give is a
    /// test that passes for a reason nobody chose.
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

        // And the bytes actually arrived, so this is a copy rather than a
        // counter with nothing behind it.
        assert_eq!(region.get8(SCRATCH_TO), Ok(0xA5));
        assert_eq!(region.get8(SCRATCH_TO + SCRATCH_BYTES - 1), Ok(0xA5));
    }

    #[test]
    fn a_copy_past_the_region_is_refused_and_counts_nothing() {
        let mut owned = Owned::new();
        let region = owned.region();
        let mut tally = 0u64;
        assert!(stage(&region, 0, CONTROL_BYTES - 8, 64, &mut tally).is_err());
        assert_eq!(tally, 0, "a refused copy is not a copy");
    }

    #[test]
    fn a_zeroed_entry_names_no_operation() {
        // The reason the opcodes start at one. An entry that was memset — a
        // slot off a free list, a peer that zeroed one — must not read as a
        // transfer of sector zero.
        assert!(!op::known(0));
        assert_eq!(
            envelope(&Sqe::ZERO),
            Err((error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE), 0))
        );
    }

    #[test]
    fn the_envelope_is_refused_before_the_opcode_is_believed() {
        let reserved = error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO);
        let unknown_flag = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);

        let mut entry = read(1, 0, 512);
        assert_eq!(envelope(&entry), Ok(()));

        let mut malformed = entry;
        malformed._reserved = 0xDEAD_BEEF;
        assert_eq!(envelope(&malformed), Err((reserved, 0xDEAD_BEEF)));

        let mut flagged = entry;
        flagged.flags |= 1 << 7;
        assert_eq!(envelope(&flagged), Err((unknown_flag, 1 << 7)));

        // Both at once: the reserved word first, because an entry with one is
        // malformed whatever else it says.
        let mut both = entry;
        both._reserved = 1;
        both.flags |= 1 << 6;
        assert_eq!(envelope(&both), Err((reserved, 1)));

        // A field this opcode does not read. `cap` belongs to a registration
        // and a transfer that carried one is a client that reused an entry.
        entry.cap = 3;
        assert_eq!(envelope(&entry), Err((reserved, 3)));
    }

    #[test]
    fn an_entry_this_service_builds_round_trips_through_its_own_envelope() {
        let asked = write(7, 4096, 1024);
        assert_eq!(asked.opcode, op::WRITE);
        assert_eq!(asked.user_data, 7);
        assert_eq!(asked.offset, 4096);
        assert_eq!(asked.len, 1024);
        assert_eq!(envelope(&asked), Ok(()));
        assert!(!opcode::is_registration(asked.opcode), "and it is not a registration");

        let asked = read(8, 0, 512);
        assert_eq!(asked.opcode, op::READ);
        assert_eq!(envelope(&asked), Ok(()));
    }
}
