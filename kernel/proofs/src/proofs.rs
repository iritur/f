// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The five properties, as bounded proofs.
//!
//! Each harness below states one of the five sentences
//! `cap::properties::Property` names, and states it the way the negative suite
//! cannot: with a solver asked whether a counterexample exists rather than
//! with a list of inputs somebody thought of.
//!
//! # The three quantifiers, and which is bounded
//!
//! **The handle is not bounded.** Every harness that answers a handle answers
//! `Handle::from_bits(kani::any())` — thirty-two symbolic bits, so all four
//! billion of them, including the ones no component could hold.
//!
//! **The rights are not bounded.** [`narrowing`] quantifies over all 256 held
//! bitmaps and all 256 asked ones, undefined bits included, which is the whole
//! lattice rather than the six pairs a test would pick.
//!
//! **The table's contents are bounded, and by construction.** A harness never
//! writes a slot. It runs `grant`, `derive`, `grow`, `condemn` and `sweep`
//! with symbolic operands, so every state a proof holds for is a state the
//! table can actually reach — which is the failure mode of proving something
//! about a structure a harness built by hand, and the reason these do it the
//! long way.
//!
//! The size bound is `mem::FRAME_SIZE` and RFC 0053 argues it.
//!
//! # Why the assertions are two-sided
//!
//! Every property here is written as an *equivalence* — `is_ok()` equals *this
//! handle was issued* — rather than as *a bad handle is refused*. A one-sided
//! property is satisfied by a table that refuses everything, which is exactly
//! what several of these would become if a `capacity` went to zero or a
//! generation stopped being issued. `cap::properties` learned that from its
//! fixtures; a proof can have it for nothing.

use f_abi::cap::{CapType, Handle, rights};
use f_abi::error;

use crate::cap::{MAX_PAGES, SLOTS_PER_PAGE, TABLE_SLOTS, Table};
use crate::mem::FRAME_SIZE;
use crate::pages::Pages;

/// The handle named nothing this table ever held.
const NO_SUCH: i32 = error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP);
/// The handle named something that has been withdrawn.
const REVOKED: i32 = error::pack(error::AUTHORITY, error::authority::REVOKED);

/// A physical address to hang a capability on. Nothing dereferences it: these
/// proofs are about the table, and `pages::Pages` is where memory that gets
/// written comes from.
const OBJECT: u64 = 0x0000_0000_1000_0000;
/// A second one, distinct so that a table answering for another's slot shows
/// up in the object it reports rather than only in a return code.
const OTHER: u64 = 0x0000_0000_2000_0000;

/// A rights bitmap this build defines, chosen by the solver.
///
/// **Not for quantifying over the lattice.** The `assume` excludes the
/// undefined bits, which is 64 of the 256 bitmaps, so a harness that took its
/// *subject* from here would be proving a quarter of what its comment said.
/// [`narrowing`] therefore does not: it takes `held` from `kani::any()`
/// directly, and costs no more for it.
///
/// What this is for is the capability a harness has to *hold* before it can
/// ask a question about some other handle — `unnamed`, `forged` and `hostile`
/// grant with it. There the bitmap is scenery rather than subject, and a
/// well-formed one is what makes the grant a grant.
fn some_rights() -> u8 {
    let bits: u8 = kani::any();
    kani::assume(!rights::unknown(bits));
    bits
}

// ---------------------------------------------------------------------------
// 1. A process cannot name a capability it was not given.
// ---------------------------------------------------------------------------

/// Two tables, and the same integer means different things in each.
///
/// The interesting form of *unnamed* is not "an empty slot is refused" — that
/// is [`forged`] — but that a handle is not a global name. The suite checks it
/// for the two handles it happened to mint; this checks it for every handle
/// there is, against a second table whose contents are symbolic.
#[kani::proof]
#[kani::unwind(34)]
fn unnamed() {
    let mut a = Table::EMPTY;
    let mut b = Table::EMPTY;
    let held = some_rights();

    let first = a.grant(CapType::Frame, held, OBJECT, 0).unwrap();
    let second = a.grant(CapType::Frame, held, OBJECT, 0).unwrap();
    let theirs = b.grant(CapType::Frame, held, OTHER, 0).unwrap();

    // B was given one capability, so A's second handle names a slot B never
    // filled — whatever the integer happens to be.
    assert_eq!(b.inspect(second), Err(NO_SUCH));
    // And where the integers do coincide, the handle resolves in B to B's
    // capability rather than to A's.
    assert_eq!(first, theirs);
    assert_eq!(b.inspect(first).map(|found| found.object), Ok(OTHER));

    // The general statement the two above are instances of: an arbitrary
    // handle resolves in B exactly when B issued it.
    let handle = Handle::from_bits(kani::any());
    assert_eq!(b.inspect(handle).is_ok(), handle == theirs);
}

// ---------------------------------------------------------------------------
// 2. A process cannot forge a handle.
// ---------------------------------------------------------------------------

/// Exactly the issued handles resolve, and nothing else does.
///
/// `properties::forged` sweeps `capacity × 8` handles and reports that none of
/// the unissued ones resolved. This says the same sentence with the sweep
/// removed: for **all** thirty-two-bit handles, `inspect` succeeds if and only
/// if the handle is one of the two the table issued.
#[kani::proof]
#[kani::unwind(34)]
fn forged() {
    let mut table = Table::EMPTY;
    let held = some_rights();
    let extent: u64 = kani::any();

    let one = table.grant(CapType::Frame, held, OBJECT, 0).unwrap();
    let two = table.grant(CapType::Untyped, held, OTHER, extent).unwrap();

    let handle = Handle::from_bits(kani::any());
    assert_eq!(table.inspect(handle).is_ok(), handle == one || handle == two);
}

/// A handle held across the end of a process resolves in the next one as
/// *revoked*, and never as itself.
///
/// The generation is the whole of this, and the failure it prevents is the one
/// E0-B10 closed: a table that reset generations with its contents would hand
/// the next occupant of a core a handle the last one is still holding. The
/// suite checks it for the handles it minted; this checks that after the
/// boundary **nothing at all** resolves, over every handle there is.
#[kani::proof]
#[kani::unwind(34)]
fn forged_across_a_process() {
    let mut table = Table::EMPTY;
    let held = some_rights();
    let one = table.grant(CapType::Frame, held, OBJECT, 0).unwrap();

    table.clear_all();

    // Named as withdrawn rather than as unknown: a component recovers from the
    // two differently, and RFC 0010 is why they are separate codes at all.
    assert_eq!(table.inspect(one), Err(REVOKED));

    let handle = Handle::from_bits(kani::any());
    assert!(table.inspect(handle).is_err());
}

// ---------------------------------------------------------------------------
// 3. A process cannot use a revoked handle.
// ---------------------------------------------------------------------------

/// Revocation is complete through the derivation tree, and it spares exactly
/// the capability it was given.
///
/// Three deep, because a revoke that stops at the children is the mistake that
/// looks like it worked. What the proof adds to the suite's version is the
/// last line: after the sweep, of all four billion handles, the only one that
/// still resolves is the one revoke was told to spare. A revocation that left
/// *anything* standing — a descendant, a slot the walk skipped, a generation
/// that failed to advance — is a counterexample to that, and the suite's three
/// named handles are not.
#[kani::proof]
#[kani::unwind(34)]
fn stale() {
    let mut table = Table::EMPTY;
    let mut pages = Pages::new();
    let full = rights::READ | rights::DERIVE | rights::REVOKE;

    let parent = table.grant(CapType::Frame, full, OBJECT, 0).unwrap();
    let child = table.derive(parent, full, &mut pages).unwrap();
    let grandchild = table.derive(child, full, &mut pages).unwrap();

    let condemned = table.condemn(parent).unwrap();
    assert_eq!(table.sweep(&condemned), 2);

    assert_eq!(table.inspect(child), Err(REVOKED));
    assert_eq!(table.inspect(grandchild), Err(REVOKED));
    assert!(table.inspect(parent).is_ok());

    let handle = Handle::from_bits(kani::any());
    assert_eq!(table.inspect(handle).is_ok(), handle == parent);
}

// ---------------------------------------------------------------------------
// 4. A process cannot exceed granted rights.
// ---------------------------------------------------------------------------

/// Derivation weakens and never widens, over the whole rights lattice.
///
/// The suite checks six pairs. This checks all 65 536: for every bitmap a
/// parent could hold and every bitmap a child could ask for, a derive succeeds
/// **if and only if** the asked bits are defined, the parent carries `DERIVE`,
/// and the asked bits add nothing — and when it succeeds the child carries
/// exactly what was asked and nothing more.
///
/// The *if* half is the one a test almost never states and the one that
/// matters: a table that refused every derive would satisfy "rights never
/// widen" completely.
#[kani::proof]
#[kani::unwind(34)]
fn narrowing() {
    let mut table = Table::EMPTY;
    let mut pages = Pages::new();

    // Both unconstrained, which is what makes the comment above true rather
    // than nearly true. `some_rights` would exclude the undefined bits and
    // leave this quantifying over 64 x 256 while the doc above said 65 536;
    // the whole lattice includes the parent holding a bit this build does not
    // define, which is exactly the parent a future entry in `rights::ALL`
    // produces. And the reduction was buying nothing: this verifies in 144 s
    // against the 150 s recorded for the constrained version when the crate
    // was written — two container wall clocks a few per cent apart, which is
    // context and not a claim, and is the reason there was never a trade here.
    let held: u8 = kani::any();
    let asked: u8 = kani::any();

    let parent = table.grant(CapType::Frame, held, OBJECT, 0).unwrap();
    let legal = !rights::unknown(asked)
        && rights::holds(held, rights::DERIVE)
        && rights::narrows(held, asked);

    match table.derive(parent, asked, &mut pages) {
        Ok(child) => {
            assert!(legal);
            let found = table.inspect(child).unwrap();
            assert_eq!(found.rights, asked);
            assert!(rights::narrows(held, found.rights));

            // And one step further down, because the mistake a table makes
            // here is checking against the original grant rather than against
            // the immediate parent — which looks right until a right dropped
            // in the middle is recovered at the bottom.
            let again: u8 = kani::any();
            if let Ok(grandchild) = table.derive(child, again, &mut pages) {
                let below = table.inspect(grandchild).unwrap();
                assert!(rights::narrows(asked, below.rights));
                assert!(rights::narrows(held, below.rights));
            }
        }
        Err(_) => assert!(!legal),
    }
}

// ---------------------------------------------------------------------------
// 5. A process cannot make the kernel panic by trying.
// ---------------------------------------------------------------------------

/// Totality, split by operation, over every handle a process could write down.
///
/// This is the property that has no fixture, and it is the reason this crate
/// exists. The suite's `total` tries nine handles somebody chose; the mutation
/// harness boots a kernel with the bounds check taken out and requires the
/// machine to die. Neither is a statement about *all* handles, and these are:
/// Kani proves the absence of a panic on every reachable path, so an index that
/// reached past the table, an arithmetic that wrapped, or an `unwrap` on a
/// lookup a hostile handle can fail is a counterexample the solver finds rather
/// than a case a fuzzer might.
///
/// # Why four harnesses and not one
///
/// This was one harness first, listing all nine operations, and it did not
/// finish inside twenty-five minutes. The reason is worth writing down because
/// it is a fact about bounded model checking rather than about this table: an
/// assertion does not cut a path. Every operation after the first still runs on
/// every path the first one produced, so nine lookups over one symbolic handle
/// multiply rather than add, and the last few operations are being proved
/// against a state space that has nothing to do with them.
///
/// Split by operation, each harness is a handful of seconds and the union of
/// the four is the same sentence. What is lost is exactly the interaction
/// between operations — *nine hostile calls in a row* is not proved, only nine
/// hostile calls each from a table two grants deep. The negative suite runs
/// them in sequence at every boot, which is the half that covers it.
///
/// # The list is the property
///
/// Every public operation on a `Table` that takes a `Handle` appears in one of
/// the four: `inspect` and `invoke`, `derive`, `condemn`, `condemn_own` and
/// `relinquish`, `note_mapping`, `note_peer_gone`, `refund`, and
/// `next_stop_notice` — which is the one that takes a handle and never
/// resolves it, and is in the list anyway because the property is *no
/// operation taking a handle panics* rather than *no lookup does*.
///
/// Adding an operation to `cap.rs` and to none of these is the way this proof
/// goes quietly narrower, which is worth knowing when reading them. The
/// sentence is checkable by grep — `pub fn` in `cap.rs` with a `Handle`
/// parameter — and that is the only reason it is worth stating as one.
fn hostile() -> (Table, Pages, Handle, Handle) {
    let mut table = Table::EMPTY;
    let pages = Pages::new();
    let full = rights::ALL & !rights::EXECUTE;

    let live = table.grant(CapType::Frame, full, OBJECT, 0).unwrap();
    let account = table.grant(CapType::Untyped, full, OTHER, FRAME_SIZE * 2).unwrap();
    (table, pages, live, account)
}

/// A handle this table did not issue, and every one of them.
fn stranger(live: Handle, account: Handle) -> Handle {
    let handle = Handle::from_bits(kani::any());
    kani::assume(handle != live && handle != account);
    handle
}

/// The two operations that only look a capability up.
///
/// The one the deliberate defect has to break: `Table::resolve` is where the
/// index is checked, and `mutate-unchecked-index` is that check removed.
#[kani::proof]
#[kani::unwind(34)]
fn total_lookup() {
    let (table, _pages, live, account) = hostile();
    let handle = stranger(live, account);
    // The rights and the type an `invoke` is asked for. `asked` is the
    // component's — it arrives in a register — so it is symbolic; the type is
    // the frame's own call site and is not.
    let asked: u8 = kani::any();

    assert!(table.inspect(handle).is_err());
    assert!(table.invoke(handle, CapType::Frame, asked).is_err());

    // And the capability the table does hold is still usable, so the two above
    // are refusals rather than a table that has stopped answering.
    assert!(table.inspect(live).is_ok());
}

/// Minting, over every handle and every rights bitmap at once.
#[kani::proof]
#[kani::unwind(34)]
fn total_derive() {
    let (mut table, mut pages, live, account) = hostile();
    let handle = stranger(live, account);
    let asked: u8 = kani::any();

    assert!(table.derive(handle, asked, &mut pages).is_err());
    assert!(table.inspect(live).is_ok());
}

/// The three that withdraw authority.
#[kani::proof]
#[kani::unwind(34)]
fn total_revoke() {
    let (mut table, _pages, live, account) = hostile();
    let handle = stranger(live, account);

    assert!(table.condemn(handle).is_err());
    assert!(table.condemn_own(handle).is_err());
    assert!(table.relinquish(handle).is_err());
    assert!(table.inspect(live).is_ok());
}

/// The four the frame performs on a component's behalf.
///
/// A hostile handle reaches these because the frame resolves one it was handed
/// — a mapping it has just made, a peer that has just died, an account it is
/// refunding — and a frame bug is as capable of producing a handle that names
/// nothing as a component is. `cap.rs` refuses rather than asserting at each of
/// them, and this is that refusal over every handle.
///
/// The fourth, `next_stop_notice`, refuses nothing: it takes a handle in order
/// to echo it and never resolves one. It is here because leaving it out would
/// make *every operation a handle can be presented to* false the day it was
/// written, and a list that is the property has to be the whole list.
#[kani::proof]
#[kani::unwind(34)]
fn total_frame_side() {
    let (mut table, _pages, live, account) = hostile();
    let handle = stranger(live, account);

    assert!(table.note_mapping(handle, OTHER).is_err());
    assert!(table.note_peer_gone(handle).is_err());
    assert!(table.refund(handle, FRAME_SIZE, OTHER).is_err());

    // The fourth takes a handle and never resolves one: it drains a deadline
    // this table was never given and echoes the bits back into a completion.
    // So there is no refusal to assert and the assertion is Kani's own — that
    // no path through it panics — which is the property this harness is about
    // and the reason an operation with nothing to refuse still belongs here.
    let _ = table.next_stop_notice(handle, 0);

    assert!(table.inspect(live).is_ok());
}

// ---------------------------------------------------------------------------
// The same five, on a table that has bought part of itself.
// ---------------------------------------------------------------------------

/// A bought table is still total, and it is bigger.
///
/// RFC 0029 made `capacity` a number the component chooses, so every bound in
/// `cap.rs` moved from a constant to a quantity. This is that half: grow the
/// table out of a real account through the real `grow`, check that it grew,
/// and then quantify over every handle again — including the ones that now
/// land in the bought page, and the ones just past it that a mask would fold
/// back into it.
#[kani::proof]
#[kani::unwind(45)]
fn total_bought() {
    let mut table = Table::EMPTY;
    let mut pages = Pages::new();
    let full = rights::READ | rights::DERIVE | rights::REVOKE;

    let account =
        table.grant(CapType::Untyped, full, OBJECT, FRAME_SIZE * MAX_PAGES as u64).unwrap();
    assert_eq!(table.capacity(), TABLE_SLOTS);
    table.grow(&mut pages).unwrap();

    // A table that had quietly stopped growing would satisfy everything below
    // by being the size the free part already was.
    assert_eq!(pages.handed(), 1);
    assert_eq!(table.capacity(), TABLE_SLOTS + SLOTS_PER_PAGE);

    // Two operations and not nine, for the reason the split above gives: after
    // the first, the rest are being proved against a state space they had no
    // part in producing. `total_lookup` and the three beside it are where the
    // other seven are, at the size a table starts at.
    let handle = Handle::from_bits(kani::any());
    kani::assume(handle != account);
    assert!(table.inspect(handle).is_err());
    assert!(table.derive(handle, rights::NONE, &mut pages).is_err());
}

// ---------------------------------------------------------------------------
// What is *not* proved here, stated where the harness would have been.
// ---------------------------------------------------------------------------
//
// **A slot in a bought page, used and given back, is not proved unreachable by
// the handle it answered to last time.** That is the half of `forged` growth
// added — `Slot::fresh` starts a bought page at the table's generation floor
// rather than at one, precisely so that the next occupant of a core cannot be
// handed a handle the last one still holds — and it is the one property in this
// file with a harness that was written, run, and then taken out.
//
// The reason is arithmetic rather than taste. Reaching a bought slot through
// the public interface means filling the free part first, because `place` fills
// the lowest vacancy; that is `TABLE_SLOTS` grants, each scanning a forty-slot
// `vacancy`, and then a `clear_all`, a second account, a second `grow` and a
// symbolic handle resolved against a page reached through a raw pointer. It did
// not terminate in forty-five minutes on the development container. Every other
// harness here is minutes or seconds, and a harness that has to be given three
// quarters of an hour is one whose failure a person will learn to wait out.
//
// What covers it instead: `cap::properties::forged` does exactly this at every
// boot, at the real page size, on a real table grown out of a real frame — see
// its second half, which mints until a handle lands past `TABLE_SLOTS`. So the
// property is checked; it is not proved. That is a smaller gap than it sounds,
// because what a proof would add over that check is quantification over the
// *handle*, and `total_bought` already quantifies over every handle against a
// grown table. What is missing is the two together.
//
// *What would close it:* `place` gaining a cheaper way to reach a bought slot,
// or `TABLE_SLOTS` shrinking, or a checker that summarises a loop instead of
// unrolling it. RFC 0053.
