// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The kernel's half of the doorbell: a vector, and something to count.
//!
//! `f_ring::doorbell` owns the protocol — when to ring, and the two counts that
//! make it measurable. This owns the one thing a ring cannot: an interrupt
//! actually arriving somewhere.
//!
//! # Why this is the third vector in the table, and not a mailbox
//!
//! The shootdown vector next door carries information — which page, and which
//! sequence number — so `smp` has two words per core for it, and `RFC 0016`
//! counts them. A doorbell carries nothing. The entry is already in the ring
//! and the cursor that publishes it is already visible; the whole content of
//! the signal is *stop halting*. So this needs no shared word, and the count
//! below is a per-core counter that only its own core writes.
//!
//! That is worth stating rather than assuming, because the obvious design —
//! a **shared** "doorbells pending" word per core — would have been a fifth
//! address two cores reach, and `CLAUDE.md` says a fifth needs an argument. It
//! does not need one because it does not exist. [`PENDING`] below is a pending
//! word and is not that word: it is written and read only by the core it names,
//! which is what the whole of `percpu.rs` is, and the distinction is the one
//! RFC 0016 draws — the rule is about slots two cores reach, not about slots.
//!
//! # What is proven at boot, and what is not
//!
//! Delivery is proven **to this core**: the boot self-test sends the vector to
//! the core it is running on and requires the count to advance. That exercises
//! the whole path — the interrupt command register, the delivery, the gate, the
//! handler, the acknowledgement — with one thing left out, which is the second
//! core.
//!
//! Cross-core delivery used to be unproven here, and the entry that deferred it
//! said why: the only way to observe another core's count is to read another
//! core's slot, which would be the fifth cross-core address the paragraph above
//! is glad not to need. `E3-B01g` is the task that owns it, and the answer it
//! found is that **no fifth word was needed**. The reasoning is
//! [`delivered_at`]'s and is the one `smp`'s own shootdown counters already
//! make: a counter read across a core boundary *after that core has reported
//! finished through the mailbox* is not a slot two cores reach, because the
//! mailbox's `Release` store and the reader's `Acquire` load are what order it.
//! The rendezvous was already there.
//!
//! # What this file gained with the sleeper, and why the latch is not optional
//!
//! Until `E3-B01g` nothing in this tree slept, so a doorbell had nowhere to ring
//! and this file only counted. [`wait`] is the other half: the frame's answer to
//! `f_abi::door::WAIT`, which stops the core a component is on until a doorbell
//! arrives.
//!
//! [`PENDING`] is what makes that safe, and it is the same defect RFC 0020
//! closed one layer up wearing different clothes. The ring's protocol has the
//! consumer arm a flag, look again, and only then decide to sleep — but *decide
//! to sleep* and *be stopped* are two instructions apart, and the producer's
//! doorbell can land between them. At ring 3 it lands as an interrupt that is
//! taken, counted and returned from, and then the core halts holding work
//! nobody will ring for again. A latch turns that into an answer: every doorbell
//! sets it, and a wait that finds it set clears it and does not halt. It costs
//! one word per core, written and read only by that core.
//!
//! *Reversal:* a wait that takes a deadline, at which point the latch stays and
//! the unbounded halt goes. `f_abi::door::WAIT` says what bounds it today.

use core::sync::atomic::{AtomicU64, Ordering, compiler_fence};

use f_ring::Ringer;

use crate::arch::x86_64::{ap, apic, current_cpu};
use crate::percpu::PerCpu;

/// Doorbells delivered to each core.
///
/// Written by the handler on the core it was delivered to, read by that same
/// core, and — once, after that core has reported finished — by the boot
/// processor. Every access goes through an atomic, which is what makes the last
/// of those three legal rather than merely rare: see [`delivered_at`].
///
/// It used to be volatile on both sides, which was right while the only reader
/// was the core that wrote it. A volatile read says *do not elide this access*
/// and says nothing about what another core may be doing to the same word at the
/// same time, and a stray doorbell delivered to a core the boot processor is
/// reading is exactly that. Unit: doorbells.
static DELIVERED: PerCpu<u64> = PerCpu::new(0);

/// A doorbell arrived on this core and no [`wait`] has consumed it yet.
///
/// One word per core, set by the handler and cleared by the wait, both on the
/// core it belongs to. **Not a cross-core word**: no other core ever reads it,
/// and the file comment says what it is for.
///
/// A count would say more and is deliberately not what this is: two doorbells
/// between two waits are one reason not to halt, and a counter would make the
/// second wait skip a halt it should have taken — which is a component spinning
/// in its idle loop for a signal that has already been answered.
/// Unit: none — a latch.
static PENDING: PerCpu<u64> = PerCpu::new(0);

/// Times [`wait`] really stopped this core. Unit: halts.
static PARKS: PerCpu<u64> = PerCpu::new(0);

/// Halts of this core that a doorbell ended, as distinct from any other
/// interrupt.
///
/// **The number the exit is about, and the reason it is a separate count from
/// [`PARKS`].** A halted core is restarted by *any* unmasked interrupt, and the
/// timer is armed on every core running a process — so a halt that ended is not
/// evidence that a doorbell arrived. This counts only the halts across which
/// [`DELIVERED`] moved, which is this core's own reading of its own counter
/// either side of one `hlt`. Unit: halts.
static WOKEN: PerCpu<u64> = PerCpu::new(0);

/// Waits that did not halt because [`PENDING`] was already set. Unit: waits.
///
/// Published rather than folded into [`PARKS`] because it is the count of times
/// the race the latch exists for actually happened, and a build whose latch had
/// stopped working would report zero here and hang rather than reporting zero
/// here and being fine. It is read beside the others, which is what makes the
/// zero readable.
static SPARED: PerCpu<u64> = PerCpu::new(0);

/// Answer a doorbell: count it, and say it is over.
///
/// There is nothing else to do. The signal's entire content is that an
/// interrupt arrived, and its effect is that a halted core is no longer halted
/// — which has already happened by the time this runs.
///
/// # Safety
///
/// Call from the doorbell vector's own gate, on the core it was delivered to,
/// with interrupts disabled by that gate.
pub(crate) unsafe fn answer() {
    let me = current_cpu();
    let seen = load(&DELIVERED, me);
    store(&DELIVERED, me, seen.wrapping_add(1));

    // The latch, after the count and before the acknowledgement. After the
    // count, so that a wait which observes the latch is entitled to the reading
    // that goes with it; before the acknowledgement, so that the local APIC
    // cannot deliver a second doorbell into a core whose first one is not
    // recorded yet.
    store(&PENDING, me, 1);

    // SAFETY: this core, inside the handler for the interrupt being
    // acknowledged. Last, so that the count is written before the local APIC
    // will deliver another.
    unsafe { apic::end_of_interrupt() };
}

/// Doorbells this core has been delivered. Unit: doorbells.
#[must_use]
pub fn delivered() -> u64 {
    load(&DELIVERED, current_cpu())
}

/// Doorbells `cpu` has been delivered. Unit: doorbells.
///
/// # Why this is not the fifth cross-core word
///
/// Because it is the shape `smp`'s shootdown counters already argued for and not
/// a new one. RFC 0016's rule is about *slots two cores reach* — addresses whose
/// correctness depends on an ordering between two running cores — and this is
/// not one: the slot is written only by the core it names, and the only reader
/// from anywhere else is a boot processor that has already seen that core report
/// finished through the mailbox. The mailbox's `Release` store and this core's
/// `Acquire` load are the happens-before, and they exist for the job they were
/// built for; nothing here adds an edge or needs one.
///
/// So the ordering on the access itself is `Relaxed`, and that is a statement
/// rather than a shortcut: an ordering here would be naming an edge nothing
/// depends on, which `smp`'s own counter bump says is worse than none in a file
/// whose argument is that every ordering in it is load-bearing.
///
/// **What is left is a race rather than a rule**, and it is why the accesses are
/// atomic at all: a doorbell delivered to `cpu` *after* it reported finished
/// would have the handler writing this word while the boot processor reads it.
/// Nothing in this tree rings a core that has ended — a boot stops ringing
/// before it joins — but *nothing does it today* is not a memory model, and an
/// atomic makes the answer a stale count instead of undefined behaviour.
///
/// What this is not is *meaningful* at any time: a reading taken while `cpu` is
/// running is a reading of a number that is moving, and every caller in this tree
/// takes it after `smp::join_serviced` or `smp::run_on` has returned `Ok`.
#[must_use]
pub fn delivered_at(cpu: usize) -> u64 {
    load(&DELIVERED, cpu)
}

/// Times `cpu` was really stopped by a wait. See [`delivered_at`].
/// Unit: halts.
#[must_use]
pub fn parks_at(cpu: usize) -> u64 {
    load(&PARKS, cpu)
}

/// Halts of `cpu` that a doorbell ended, as distinct from any other interrupt.
/// See [`delivered_at`]. Unit: halts.
#[must_use]
pub fn woken_at(cpu: usize) -> u64 {
    load(&WOKEN, cpu)
}

/// Waits on `cpu` that found a doorbell already latched and did not halt.
/// See [`delivered_at`]. Unit: waits.
#[must_use]
pub fn spared_at(cpu: usize) -> u64 {
    load(&SPARED, cpu)
}

/// Stop this core until a doorbell arrives, and say whether it really stopped.
///
/// Answers `f_abi::door::HALTED` or `f_abi::door::AWAKE`, which is the whole of
/// what a component learns. The four counters this moves are the frame's, and
/// they are read from the boot processor after the core comes back.
///
/// # The two instructions that have to be one, and why
///
/// `sti` followed by `hlt` is the architecture's own idiom and not a pair that
/// happens to work: `sti` opens the interrupt window only *after* the
/// instruction that follows it, so an interrupt that arrives in between is taken
/// after the `hlt` has been entered rather than before it. Written apart — or
/// with anything between them — this is a core that halts with a doorbell
/// already asserted and does not come back.
///
/// The window before that is closed by the caller rather than here: a system
/// call runs with interrupts masked by `IA32_FMASK`, so a doorbell arriving
/// between the latch being read and the `sti` is held in the local APIC's
/// request register and delivered the instant the window opens. That is the one
/// obligation this function has of its caller and it is in the safety section.
///
/// # Safety
///
/// Call on the core the component is running on, from the system-call path,
/// with interrupts masked by `IA32_FMASK` and with nothing of the calling core's
/// process state held across the call — interrupts are enabled inside, and the
/// timer handler writes the same shards `process::syscall` reads.
pub(crate) unsafe fn wait() -> i64 {
    let me = current_cpu();

    // The latch first, and cleared whether or not it was set. A wait that found
    // it set has been told *there was already something for you*, and leaving it
    // set would make the next wait skip a halt it should take.
    let latched = load(&PENDING, me);
    store(&PENDING, me, 0);
    if latched != 0 {
        store(&SPARED, me, load(&SPARED, me).wrapping_add(1));
        return f_abi::door::AWAKE;
    }

    let before = load(&DELIVERED, me);
    store(&PARKS, me, load(&PARKS, me).wrapping_add(1));

    // SAFETY: the caller's guarantee that this is the system-call path with
    // interrupts masked. Every vector the local APIC can deliver to this core
    // has a gate — `idt::init` ran on it at bring-up — and none of them reads
    // `GS`, which is what makes taking one at ring 0 on this path survivable:
    // the system-call entry has already swapped it and the interrupt stubs do
    // not. `cli` on the way out so that the rest of the call returns under the
    // condition it was entered with.
    //
    // `nostack` is deliberately **not** claimed. This block does not push
    // anything itself, but it is the one block in this kernel that is entered
    // with interrupts about to be enabled, and an interrupt taken inside it
    // lands on this core's kernel stack. The option's real cost is the red zone,
    // which `x86_64-unknown-none` disables anyway — so claiming it would buy
    // nothing and would put a promise here that depends on a target setting
    // rather than on this code. `preserves_flags` *is* claimed and is true:
    // `sti` and `cli` move the interrupt flag, which is not among the status
    // flags that option is about, and the block leaves it as it found it.
    unsafe {
        core::arch::asm!("sti", "hlt", "cli", options(preserves_flags));
    }

    // Read after the halt and compared against the reading before it. This is
    // the whole of how a doorbell is told apart from the timer, which is armed
    // on every core running a process and would otherwise make every halt look
    // like a delivery.
    if load(&DELIVERED, me) != before {
        store(&WOKEN, me, load(&WOKEN, me).wrapping_add(1));
    }

    // **And the latch cleared again, because this wait has consumed whatever
    // set it.** A doorbell that ended the halt ran [`answer`], which latched —
    // and until RFC 0137 that latch outlived the halt it had already ended, so
    // the component's *next* wait found it set and was spared a halt nobody had
    // rung for. Every doorbell-ended halt was followed by one false spare: the
    // boot printed eight or nine spares against ten deliveries on every passing
    // run, which is `E3-B01g`'s *the latch fires nine times* read as the race
    // when it was mostly this. Cleared, the count fell to between none and four. It also made the ask the wake half's
    // client waits on a spared one most of the time, so the next submission
    // landed on a running core rather than a stopped one.
    //
    // Clearing it here loses nothing, and the argument is the component's loop
    // rather than this function: every doorbell counted before this line — the
    // one that ended the halt, or one that arrived before `cli` — precedes the
    // component's return from this call, and the component looks at its ring
    // after every return. A doorbell arriving after `cli` is held by the local
    // APIC until the return to ring 3 re-enables interrupts, latches then, and
    // spares the next wait, which is the latch doing its job. *What would
    // reverse this:* a waiter that does not look at its ring after every wait,
    // for which a consumed doorbell and a pending one would differ.
    store(&PENDING, me, 0);
    f_abi::door::HALTED
}

/// Read one core's word out of a shard, atomically.
///
/// `smp`'s equivalent makes the same argument, for the same reason and with one
/// difference worth naming: there the atomic is what makes the *other* core's
/// access legal, and here it is what makes the *handler's* access legal beside
/// the code it interrupted. Both are cases where a reference may not be taken,
/// and volatile is the wrong tool for the second one now that a word in this
/// file is read from somewhere else at all.
fn load(shard: &'static PerCpu<u64>, cpu: usize) -> u64 {
    let slot = shard.at(cpu);
    // SAFETY: `at` returns a pointer to one aligned `u64` inside a `'static`
    // shard, which is a valid `AtomicU64` for as long as every access to it goes
    // through one — and in this file every access does. The reference does not
    // outlive the call.
    unsafe { AtomicU64::from_ptr(slot) }.load(Ordering::Relaxed)
}

/// Write one core's word into a shard, atomically. As [`load`].
fn store(shard: &'static PerCpu<u64>, cpu: usize, value: u64) {
    let slot = shard.at(cpu);
    // SAFETY: as [`load`].
    unsafe { AtomicU64::from_ptr(slot) }.store(value, Ordering::Relaxed);
}

/// Ring `cpu`'s doorbell.
///
/// # Safety
///
/// The local APIC must be mapped and `cpu` must be a core that is running, or
/// the interrupt is delivered to nobody and the interrupt command register is
/// left holding a command for a destination that does not answer.
pub unsafe fn ring(cpu: usize) {
    // The compiler may not move the ring above whatever published the work it
    // is announcing. There is no *hardware* ordering question here — the
    // interrupt is a message and the store buffer is drained by the write to
    // the command register — but a reordered call would announce work that has
    // not been written, and that is a compiler question.
    compiler_fence(Ordering::Release);

    // SAFETY: the caller's guarantee, and a vector `idt::init` installs on
    // every core.
    unsafe { ap::send(apic::window(), cpu, apic::DOORBELL_VECTOR) };
}

/// The doorbell as `f_ring` sees it: something with a `ring`.
///
/// Carries the core it rings, which is why the trait takes an implementor
/// rather than a function — `f_ring` has no idea what an APIC identifier is and
/// should not acquire one.
pub struct Ipi {
    target: usize,
}

impl Ipi {
    /// A doorbell that wakes `cpu`.
    #[must_use]
    pub fn to(cpu: usize) -> Self {
        Self { target: cpu }
    }

    /// A doorbell that wakes the core it is rung on.
    ///
    /// What the boot self-test uses. A self-directed inter-processor interrupt
    /// is a real delivery and not a shortcut — the command register, the local
    /// APIC's own routing, the gate and the acknowledgement are all the ones a
    /// cross-core ring would use — and it needs no second core to be running
    /// and no way to read that core's counters.
    #[must_use]
    pub fn to_self() -> Self {
        Self { target: current_cpu() }
    }
}

impl Ringer for Ipi {
    fn ring(&mut self) {
        // SAFETY: the local APIC is mapped before anything builds one of these
        // — `apic::window` is what would fault otherwise — and `target` is a
        // core this kernel started or is this one.
        unsafe { ring(self.target) };
    }
}
