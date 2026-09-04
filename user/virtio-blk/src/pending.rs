// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What the driver has been given and not yet handed to the device, and the
//! order it hands them over in.
//!
//! # Where the ordering can happen at all, and where it cannot
//!
//! A virtqueue is a ring the device consumes in the order the driver posts. So
//! there is exactly one place in this system where a block request's urgency
//! can change what happens to it, and it is **before** the descriptor goes into
//! the available ring: what the driver chooses to post next. After that the
//! request belongs to the device, and no scheduler above it — this one, the
//! client's, the frame's — can move it.
//!
//! That is a cost and it is stated as a number rather than left implied.
//! [`IN_FLIGHT`] is how many requests this driver keeps inside the device at
//! once, and it is the granularity of every overtake this module can perform: a
//! hard-class read arriving while `IN_FLIGHT` batch requests are already in the
//! device waits for all of them, whatever its deadline says. Today it is one,
//! because [`crate::driver::Driver::execute`] offers one three-descriptor chain
//! and polls the used ring until it comes back. `E1-B09` is the task that makes
//! this driver wait on its interrupt instead of spinning, and it is the change
//! that would raise this number — at which point the overtake gets coarser and
//! the claim that measures it says so, because it reads this constant.
//!
//! # What decides the order
//!
//! [`f_abi::deadline::Inherited::rank`] and nothing else. RFC 0025 decided the
//! rule and `abi/src/deadline.rs` holds it as arithmetic; this module is one of
//! the schedulers that orders by what it returns, which is the whole of what
//! `E1-B06` means by *every resource scheduler orders by the same field*. There
//! is no second policy here and there must not be one: a service that re-derived
//! the order from `Sqe::class` and `Sqe::deadline` itself would be a service
//! that had quietly opted out of the four bounds, and the way a reader checks
//! that this one has not is that [`Pending::take`] compares `rank()` and never
//! an entry.
//!
//! # Why the tie-break is arrival and why that is not a detail
//!
//! Two requests of the same class with the same deadline — which includes every
//! pair of batch requests, because batch work carries [`f_abi::NO_DEADLINE`] and
//! `rank` sorts that last within its class — are ordered by when they arrived.
//! Without that, equal-ranked work would be ordered by whatever the array
//! happened to hold, which is a scheduler whose output depends on its own
//! history of removals: reproducible, unreadable, and a starvation bug nobody
//! can reason about. With it, this queue is first-come-first-served *within a
//! rank*, which is the property that makes "the read overtook six batch
//! operations" a sentence about urgency rather than about an array.

use f_abi::deadline::{Admitted, Callee, Caller, Inherited};
use f_abi::{Sqe, error};

use f_ring::registry::Refusal;

/// How many requests this driver keeps inside the device at once.
/// Unit: requests.
///
/// One. [`crate::driver::Driver::execute`] builds a chain, rings the doorbell
/// and polls the used ring until that chain comes back, so at the moment a pick
/// is made there is never more than one request this queue could not have
/// reordered. It is published — through the routing page and into the boot log
/// — because it is the *granularity* of every overtake this module performs,
/// and a demonstration that reported an overtake without reporting the depth it
/// was performed at would be hiding the one number that bounds it. R12.
///
/// *Reversal, and it has an owner:* `E1-B09` waits on the device's interrupt
/// rather than spinning on its used ring, which is what makes more than one
/// chain outstanding worth having. When this is not one, `claims/0012`'s
/// `in_flight` threshold is what changes with it.
pub const IN_FLIGHT: u32 = 1;

/// How many requests this driver will hold before it stops taking them off the
/// ring. Unit: requests.
///
/// **Eight, and the bound is this component's stack rather than its client's
/// ring.** `kernel::process::STACK` is one page, a submission entry is
/// sixty-four bytes, and this queue is the largest thing in
/// `component::serve`'s frame — so a queue as deep as the sixteen-entry ring
/// the frame describes, laid out the obvious way as `[Option<Waiting>; 16]`,
/// overflowed into the guard page and killed the component before it answered
/// anything. That is recorded rather than quietly fixed, because the failure
/// arrived as *the driver did not answer a completion inside the bound*, which
/// is five seconds of looking at the wrong thing.
///
/// Two consequences worth stating. The queue may be shallower than the ring, so
/// a client can have entries published that this driver has not taken yet —
/// which is not a stall: they are taken on the next turn of the loop, after one
/// is served. And [`Pending`] holds its entries and its ordering in *parallel
/// arrays* rather than one array of pairs, because a submission entry is
/// sixty-four-byte aligned and anything bundled beside one rounds the pair up
/// to a hundred and twenty-eight.
///
/// *Reversal:* a component with more than a page of stack. `E1-B05`'s
/// supervisor spawns components into places with accounts sized from their own
/// manifests, and a driver that declares its stack is a driver whose queue is
/// bounded by its client's ring again.
pub const CAPACITY: usize = 8;

/// What one waiting request costs this component's stack. Unit: bytes.
///
/// A quarter of the one page `kernel::process::STACK` maps, and the assertion
/// below is what keeps it that way: this queue grew past the stack once, and a
/// constant somebody raises without re-deriving this is how it does so again.
const STACK_BUDGET: usize = 1024;

/// What this service is admitted for, what its channel says about the peer
/// submitting on it, and the least time it needs.
///
/// Told to the component rather than assumed by it, for
/// `crate::routing`'s reason: a component's ceiling is granted at spawn from
/// what its manifest declares, and a component that wrote its own down would be
/// a component that could raise it. Both halves reach this driver through the
/// routing page the frame fills in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admission {
    /// This component's own ceiling, from its manifest's `[reservation] class`.
    /// A request is never served above it. Unit: none — an `f_abi::class`
    /// ordinal.
    pub mine: Admitted,
    /// What the channel reports about whoever submits on it. Never read off an
    /// entry: RFC 0025 bound 2 exists because an entry cannot raise it.
    /// Unit: none — an `f_abi::class` ordinal.
    pub client: Admitted,
    /// The least time this component needs from arrival to completion for any
    /// request. Unit: nanoseconds.
    ///
    /// **Read and passed on, and on this boot it bounds nothing**, which is
    /// said here rather than discovered. `f_abi::deadline::inherit` floors a
    /// deadline at *arrival plus this*, and the arrival a component can supply
    /// is zero: RFC 0004 gives a component no clock, `Driver::execute` takes
    /// `now` as an argument for exactly that reason, and the only caller passes
    /// a literal zero. So bound 3 is measured from the channel epoch's origin
    /// rather than from when the entry turned up, which makes it a constant
    /// floor rather than a moving one — and an absurd deadline still sorts
    /// ahead of an honest one, which is the failure bound 3 exists to prevent.
    /// Nothing in this crate closes that; a component that can read a clock
    /// does. `DEADLINE_GAP` in `xtask` is where it is declared and what goes
    /// red the day the literal zero goes.
    pub floor: u64,
}

/// Which order the driver hands work to the device in.
///
/// Two, and the second is a **control** rather than a mode a deployment would
/// choose. `E1-B06`'s exit is that a hard-class read overtakes queued batch
/// work, and a demonstration of that with nothing to compare against is a
/// demonstration that the array happened to come out in a convenient order —
/// the same argument `kernel/src/blk.rs` makes about `inside` being worthless
/// without `outside`, and `mutate` about a suite that has never gone red. So
/// the frame can ask for either, the boot runs both, and one of them must fail
/// to overtake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// By what `inherit` returned: class first, then deadline, then arrival.
    Rank,
    /// By arrival alone. The control, and what this driver did before
    /// `E1-B06`: `driver.rs` said so in a comment, and this is that comment
    /// turned into something a boot can run.
    Arrival,
}

impl Order {
    /// From the ordinal the routing page carries. Anything that is not
    /// [`Order::Rank`]'s is the arrival order, which is R04 pointing the safe
    /// way: a routing page the frame did not fill in reads as zero, and zero
    /// must be the behaviour that claims nothing.
    #[must_use]
    pub const fn from_ordinal(value: u64) -> Self {
        if value == 1 { Self::Rank } else { Self::Arrival }
    }

    /// A word for a report.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rank => "rank",
            Self::Arrival => "arrival",
        }
    }
}

/// One request waiting for the device.
#[derive(Clone, Copy, Debug)]
pub struct Waiting {
    /// The entry as the client wrote it, carried whole: the executor reads
    /// fields this module does not, and a queue that kept only what it sorts by
    /// would be a queue that had decided what a request is.
    pub entry: Sqe,
    /// What this request is served as here, and what the completion owes the
    /// client. From `f_abi::deadline::inherit` and never recomputed.
    pub order: Inherited,
    /// Where in this queue's arrival order the request sits. Unit: requests.
    pub arrival: u64,
}

/// The ordering half of one waiting request, without its entry.
///
/// Beside [`Pending::entries`] rather than inside it, for the reason
/// [`CAPACITY`] gives: a `Sqe` is sixty-four-byte aligned, so a struct holding
/// one and anything else is a hundred and twenty-eight bytes, and this queue
/// lives on a one-page stack.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// Whether the entry at this index is a request and not the zeroes the
    /// array was built with. A flag and not an `Option`, because an `Option`
    /// around what follows costs a word this stack does not have.
    live: bool,
    /// What `f_abi::deadline::inherit` said this request is served as.
    order: Inherited,
    /// Where in this queue's arrival order it sits. Unit: requests.
    arrival: u64,
}

/// The requests taken off the ring and not yet handed to the device.
#[derive(Debug)]
pub struct Pending {
    /// The entries, whole. A queue that kept only what it sorts by would be a
    /// queue that had decided what a request is.
    entries: [Sqe; CAPACITY],
    /// What each of them is served as, and whether it is there at all.
    slots: [Slot; CAPACITY],
    /// How many requests this queue has ever taken. The arrival stamp, and the
    /// tie-break within a rank. Unit: requests.
    arrivals: u64,
    /// The most this queue ever held at once. Unit: requests.
    deepest: u32,
    /// How many requests were waiting, and had arrived earlier, when a request
    /// was picked ahead of them. Unit: requests.
    overtaken: u32,
}

const _: () = assert!(
    core::mem::size_of::<Pending>() <= STACK_BUDGET,
    "the driver's pending queue no longer fits the stack budget: see CAPACITY"
);

impl Default for Pending {
    fn default() -> Self {
        Self::new()
    }
}

impl Pending {
    /// An empty queue.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [Sqe::ZERO; CAPACITY],
            slots: [Slot { live: false, order: NOTHING, arrival: 0 }; CAPACITY],
            arrivals: 0,
            deepest: 0,
            overtaken: 0,
        }
    }

    /// How many are waiting. Unit: requests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.live).count()
    }

    /// Is nothing waiting?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.slots.iter().any(|slot| slot.live)
    }

    /// Is there no room for another?
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.slots.iter().all(|slot| slot.live)
    }

    /// The most that were ever waiting at once. Unit: requests.
    #[must_use]
    pub const fn deepest(&self) -> u32 {
        self.deepest
    }

    /// How many waiting requests have been overtaken, in total.
    ///
    /// Counted at the pick and not at the completion, and the difference
    /// matters: this is *the queue's own reading* of what its ordering did, and
    /// the client's reading is the order its completions arrive in. The two are
    /// produced on opposite sides of a privilege boundary from different
    /// evidence, and `kernel/src/blk.rs` requires them to agree — because a
    /// counter this component increments is a counter this component could be
    /// wrong about. Unit: requests.
    #[must_use]
    pub const fn overtaken(&self) -> u32 {
        self.overtaken
    }

    /// Put one at the back.
    ///
    /// # Errors
    ///
    /// `RESOURCE/QUOTA_EXHAUSTED` with [`CAPACITY`] as its detail, for a queue
    /// with no room. Refused rather than dropped and refused rather than
    /// overwriting the least urgent thing waiting: a client whose request
    /// disappeared because something more urgent turned up cannot tell that
    /// from a service that died, and R07 says it must be able to.
    pub fn push(&mut self, entry: Sqe, order: Inherited) -> Result<(), Refusal> {
        let free = self.slots.iter_mut().zip(self.entries.iter_mut()).find(|(slot, _)| !slot.live);
        let Some((slot, place)) = free else {
            return Err((
                error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                CAPACITY as u64,
            ));
        };
        *place = entry;
        *slot = Slot { live: true, order, arrival: self.arrivals };
        self.arrivals = self.arrivals.saturating_add(1);
        let held = u32::try_from(self.len()).unwrap_or(u32::MAX);
        if held > self.deepest {
            self.deepest = held;
        }
        Ok(())
    }

    /// Take the one the device should be given next, and count what it went
    /// ahead of.
    ///
    /// `None` for an empty queue.
    pub fn take(&mut self, how: Order) -> Option<Waiting> {
        let mut best: Option<Waiting> = None;
        for (slot, entry) in self.slots.iter_mut().zip(self.entries.iter()) {
            if !slot.live {
                continue;
            }
            let candidate = Waiting { entry: *entry, order: slot.order, arrival: slot.arrival };
            let better = match &best {
                None => true,
                Some(held) => key(&candidate, how) < key(held, how),
            };
            if better {
                best = Some(candidate);
            }
        }
        let taken = best?;
        // What this pick went ahead of: everything still waiting that arrived
        // first. Zero for the arrival order by construction, which is what makes
        // the control run a control rather than a second name for the same
        // thing. Counted in the same pass that frees the slot, so there is one
        // definition of *still waiting* rather than two that could drift.
        let mut behind = 0u32;
        for slot in &mut self.slots {
            if !slot.live {
                continue;
            }
            if slot.arrival == taken.arrival {
                slot.live = false;
                continue;
            }
            if slot.arrival < taken.arrival {
                behind = behind.saturating_add(1);
            }
        }
        self.overtaken = self.overtaken.saturating_add(behind);
        Some(taken)
    }
}

/// What a slot holding no request carries, so that [`Pending::new`] can be
/// `const` without an `Option` around every entry.
///
/// Never read: [`Slot::live`] is what says whether the row is a request, and
/// every path that reads the order checks it first. It is the batch class with
/// no deadline anyway, which is the least urgent thing this type can hold — so
/// a bug that read one would sort it last rather than first, which is the safe
/// direction for a value nobody meant.
const NOTHING: Inherited =
    Inherited { class: f_abi::class::BATCH, deadline: f_abi::NO_DEADLINE, depth: 0, shortfall: 0 };

/// The key one waiting request is ordered by.
///
/// The rank first and the arrival last, so that equal-ranked work is
/// first-come-first-served — see the module comment on why that is load-bearing
/// rather than tidy. Under [`Order::Arrival`] the rank is dropped and only the
/// arrival is left, which is the whole of the control: same queue, same
/// entries, same code, one term removed.
fn key(waiting: &Waiting, how: Order) -> (u16, u64, u64) {
    match how {
        Order::Rank => {
            let (class, deadline) = waiting.order.rank();
            (class, deadline, waiting.arrival)
        }
        Order::Arrival => (0, 0, waiting.arrival),
    }
}

/// Decide what one entry is served as, against what this channel was admitted
/// for.
///
/// A thin call and deliberately nothing more: the rule is
/// `f_abi::deadline::inherit`'s and a second opinion about it in a driver is
/// exactly what RFC 0025 forecloses. What this adds is the two things `inherit`
/// cannot know — which channel the entry arrived on, and when — and `now` is an
/// argument rather than a reading because this crate observes no clock.
///
/// # Errors
///
/// What `inherit` refuses: a class field naming no class or carrying a depth no
/// conforming service wrote, and a class more urgent than the submitter holds.
/// Both as `(packed, detail)`, which is what a service writes into a
/// completion.
pub fn admit(entry: &Sqe, admission: Admission, now: u64) -> Result<Inherited, Refusal> {
    f_abi::deadline::inherit(
        &Caller::of(entry, admission.client),
        Callee { admitted: admission.mine, arrival: now, floor: admission.floor },
    )
}

#[cfg(test)]
mod tests {
    use f_abi::{NO_DEADLINE, class};

    use super::*;
    use crate::driver;

    const HARD: Admitted = match Admitted::new(class::HARD) {
        Some(admitted) => admitted,
        None => panic!("HARD is a class"),
    };
    const SOFT: Admitted = match Admitted::new(class::SOFT) {
        Some(admitted) => admitted,
        None => panic!("SOFT is a class"),
    };
    const BATCH: Admitted = match Admitted::new(class::BATCH) {
        Some(admitted) => admitted,
        None => panic!("BATCH is a class"),
    };

    /// The driver's own admission on this boot: soft, which is what
    /// `user/virtio-blk/manifest.toml` declares, with a client admitted for the
    /// hard class.
    const SERVICE: Admission = Admission { mine: SOFT, client: HARD, floor: 1_000 };

    /// A batch read, which is what a compaction pass submits: no deadline, and
    /// `Sqe::ZERO` writes the class.
    fn batch(token: u64) -> Sqe {
        driver::read(token, 0, 512)
    }

    /// A hard-class read with a deadline, which is what a client blocking on a
    /// page submits.
    fn hard(token: u64, deadline: u64) -> Sqe {
        let mut entry = driver::read(token, 0, 512);
        entry.class = f_abi::deadline::pack(class::HARD, 0);
        entry.deadline = deadline;
        entry
    }

    fn queued(entries: &[Sqe], admission: Admission) -> Pending {
        let mut pending = Pending::new();
        for entry in entries {
            let order = admit(entry, admission, 0).expect("an entry this channel may submit");
            pending.push(*entry, order).expect("room");
        }
        pending
    }

    fn drained(pending: &mut Pending, how: Order) -> [u64; 7] {
        let mut order = [u64::MAX; 7];
        for slot in &mut order {
            let Some(taken) = pending.take(how) else { break };
            *slot = taken.entry.user_data;
        }
        order
    }

    #[test]
    fn a_hard_read_submitted_last_is_handed_to_the_device_first() {
        // E1-B06's exit, as the ordering it rests on. Six batch reads arrive,
        // then one hard-class read with a deadline, and the queue hands the
        // device the last arrival first — going ahead of six requests that were
        // already waiting.
        let mut entries: [Sqe; 7] = [batch(0); 7];
        for (index, entry) in entries.iter_mut().enumerate().take(6) {
            *entry = batch(index as u64);
        }
        entries[6] = hard(100, 1_000_000);

        let mut pending = queued(&entries, SERVICE);
        assert_eq!(pending.deepest(), 7);
        assert_eq!(drained(&mut pending, Order::Rank), [100, 0, 1, 2, 3, 4, 5]);
        assert_eq!(pending.overtaken(), 6, "the read went ahead of six that were already waiting");
    }

    #[test]
    fn the_same_queue_in_arrival_order_hands_it_over_last() {
        // The control, and the reason the boot runs two halves rather than one:
        // identical entries, identical queue, one term removed from the key.
        // Without this, the assertion above is satisfied by an array that
        // happened to come out that way.
        let mut entries: [Sqe; 7] = [batch(0); 7];
        for (index, entry) in entries.iter_mut().enumerate().take(6) {
            *entry = batch(index as u64);
        }
        entries[6] = hard(100, 1_000_000);

        let mut pending = queued(&entries, SERVICE);
        assert_eq!(drained(&mut pending, Order::Arrival), [0, 1, 2, 3, 4, 5, 100]);
        assert_eq!(pending.overtaken(), 0, "nothing may overtake in the arrival order");
    }

    #[test]
    fn equal_rank_is_first_come_first_served() {
        // Every pair of batch requests is an equal-ranked pair, because batch
        // work carries no deadline and `rank` sorts `NO_DEADLINE` last within
        // its class. If the tie-break were anything but arrival, this queue's
        // output would depend on its own history of removals.
        let entries: [Sqe; 4] = [batch(10), batch(11), batch(12), batch(13)];
        let mut pending = queued(&entries, SERVICE);
        let order = drained(&mut pending, Order::Rank);
        assert_eq!(&order[..4], &[10, 11, 12, 13]);
        assert_eq!(pending.overtaken(), 0);

        // And two hard-class requests with the *same* deadline are the same
        // case one class up, so the tie-break is not a property of batch work.
        let entries: [Sqe; 2] = [hard(20, 5_000), hard(21, 5_000)];
        let mut pending = queued(&entries, SERVICE);
        assert_eq!(&drained(&mut pending, Order::Rank)[..2], &[20, 21]);
    }

    #[test]
    fn the_earlier_deadline_goes_first_within_a_class() {
        // The half of the rank that is not the class. Three hard-class reads
        // arriving in the wrong order for their deadlines, and the queue puts
        // them in the right one — which is what makes the field a deadline
        // rather than a second class bit.
        let entries: [Sqe; 4] =
            [hard(30, 9_000), hard(31, 3_000), hard(32, 6_000), hard(33, NO_DEADLINE)];
        let mut pending = queued(&entries, SERVICE);
        assert_eq!(&drained(&mut pending, Order::Rank)[..4], &[31, 32, 30, 33]);
        // Two and not three, and the arithmetic is worth stating because it is
        // what the count *means*: 31 was picked with 30 still waiting behind it,
        // and 32 was picked with 30 still waiting behind it. 33 overtook
        // nothing — by the time it was picked, nothing that arrived before it
        // was still there. The count is *overtakings*, one per pick per request
        // left behind, and not a count of requests that were ever late.
        assert_eq!(pending.overtaken(), 2, "31 went ahead of 30, and then 32 did");
    }

    #[test]
    fn a_hard_read_is_served_at_this_services_class_and_says_so() {
        // R08, at the one place it is decided. This driver's manifest declares
        // the soft class, so a hard-class read is served as soft — never
        // promoted, and never quietly: `SHORTFALL` is the flag the completion
        // carries, and `fell_short` is what sets it.
        let order = admit(&hard(40, 1_000_000), SERVICE, 0).expect("a class the client holds");
        assert_eq!(order.class, class::SOFT, "the callee's class is a ceiling");
        assert!(order.fell_short(), "and being served below what was asked is reported");
        assert_eq!(order.shortfall, f_abi::deadline::shortfall::CLASS);
        assert_eq!(order.deadline, 1_000_000, "the deadline survives the demotion");

        // And it still outranks batch work, which is the point of keeping the
        // deadline rather than dropping it with the class.
        let batch_order = admit(&batch(41), SERVICE, 0).expect("batch is under every ceiling");
        assert!(order.rank() < batch_order.rank());
        assert!(!batch_order.fell_short(), "served as asked, so nothing is reported");
    }

    #[test]
    fn a_class_the_client_was_not_admitted_for_is_refused_and_never_queued() {
        // Bound 2, at a service. A channel whose submitter holds nothing above
        // batch may not write `HARD`, and the refusal is `ADMISSION`/`NOT_HELD`
        // rather than a demotion — because a caller that could write `HARD` and
        // be served anyway has nothing to lose by writing it on every entry.
        let unadmitted = Admission { client: BATCH, ..SERVICE };
        assert_eq!(
            admit(&hard(50, 1_000_000), unadmitted, 0),
            Err((
                error::pack(error::ADMISSION, error::admission::NOT_HELD),
                u64::from(f_abi::deadline::pack(class::HARD, 0))
            ))
        );
        // The same channel's ordinary work is untouched, which is what makes
        // the refusal a refusal of *this entry* and not of this client.
        assert!(admit(&batch(51), unadmitted, 0).is_ok());
    }

    #[test]
    fn a_full_queue_refuses_rather_than_dropping_what_it_holds() {
        let mut pending = Pending::new();
        let order = admit(&batch(0), SERVICE, 0).expect("batch");
        for token in 0..CAPACITY as u64 {
            pending.push(batch(token), order).expect("room for CAPACITY");
        }
        assert!(pending.is_full());
        assert_eq!(
            pending.push(hard(999, 1), order),
            Err((error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED), CAPACITY as u64)),
            "an urgent request past the bound is refused, not swapped for a waiting one"
        );
        // And nothing was lost: the queue still holds exactly what it held.
        assert_eq!(pending.len(), CAPACITY);
        assert_eq!(pending.take(Order::Rank).map(|w| w.entry.user_data), Some(0));
    }

    #[test]
    fn an_empty_queue_hands_over_nothing() {
        let mut pending = Pending::new();
        assert!(pending.is_empty());
        assert!(pending.take(Order::Rank).is_none());
        assert!(pending.take(Order::Arrival).is_none());
        assert_eq!(pending.deepest(), 0);
    }

    #[test]
    fn the_ordering_ordinal_fails_closed() {
        // A routing page the frame did not fill in reads as zero, and zero must
        // be the order that claims nothing. Anything this build does not name
        // reads the same way.
        assert_eq!(Order::from_ordinal(0), Order::Arrival);
        assert_eq!(Order::from_ordinal(1), Order::Rank);
        for unknown in [2u64, 3, u64::MAX] {
            assert_eq!(Order::from_ordinal(unknown), Order::Arrival);
        }
    }

    #[test]
    fn the_depth_this_driver_keeps_in_the_device_is_one() {
        // The number that bounds every overtake above, asserted rather than
        // described. `Driver::execute` offers one chain and polls it home
        // before it takes another, so a request already in the device is a
        // request nothing here can move — and `claims/0012` publishes this
        // constant beside the count it bounds.
        assert_eq!(IN_FLIGHT, 1);
    }
}
