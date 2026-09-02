# RFC 0008: No fork, no signals — spawn from a manifest, one control ring, the powerbox grant

- Status: accepted
- Date: 2026-09-02
- Affects: `kernel/` (`process.rs`, `cap.rs`, `ring.rs`), `abi/` (`cap.rs` —
  `Endpoint`, `Untyped` and `rights::GRANT` acquire their objects; `door.rs`,
  which shrinks; `feature::CONTROL_EVENTS`, which acquires a meaning),
  `user/init/`, `docs/design/deadline-all-the-way-down.html` sections 02 and
  07, `docs/design/fast-path.html` (the component model row),
  `docs/what-must-be-stated.html` sections 04 and 08 (the `fork()`, *Signals*
  and *Pressure is discovered, not delivered* rows), section 16 (the paragraph
  that sketched this RFC) and rules R05 and R06 in section 15; RFC 0006
  (suspend quiesces by rings), RFC 0013 (state is read, never delivered), RFC
  0014 and RFC 0015 (the door, and what retires its calls); the manifest schema
  of E1-D04 and the domain field of RFC 0005, both of which this RFC names and
  neither of which it writes; and `TODO.md` tasks E1-B05, E1-B08 and E1-B13,
  which build it.

## Decision

A component comes into existence in exactly one way, is controlled through
exactly one path, holds exactly what it was handed, and ends in a way every
other party can see. Stated so it can be disagreed with:

- **Spawn, never fork.** A component is created when a supervisor submits a
  spawn entry naming a manifest by its content hash and listing, handle by
  handle from its own table, the capabilities that satisfy what the manifest
  declares. The frame builds a fresh address space, a fresh capability table
  and one control ring, all paid from an `Untyped` capability the supervisor
  supplies, and copies the image in. Nothing crosses from the supervisor to the
  child except what is on that list: no memory, no handles, no environment, no
  identity. Rule R06.
- **One control ring, and the frame is the only thing on the other end of it.**
  Every component has one channel to the frame, created with it. The component
  submits on it — the capability operations RFC 0015 put behind the door,
  spawn, connect, stop — and the frame completes on it. Every notice the
  component ever receives — a capability arrived, a capability was withdrawn,
  a peer died, stop by this deadline, a core is being reclaimed by this
  deadline, the pressure grade changed, the generation changed — is a
  completion entry on that ring, drained at a polling point. There is no
  handler, no interrupted instruction stream, no second path in. Rule R05. A
  supervisor never speaks to its child directly; it asks the frame, and the
  frame posts the notice.
- **Authority is routed or asked for; it is never ambient.** A component holds
  what its manifest declared and its supervisor supplied at spawn, plus what it
  is granted at run time through a broker the user can see — the powerbox —
  in answer to an *ask* its manifest also declared. Every grant is a
  derivation in somebody's tree, so every grant can be taken back. There is no
  name a component can present to the frame and be given something.
- **Death is uniform, whatever caused it.** Fault, exit, or a supervisor's stop
  whose deadline passed: the frame revokes everything in the component's table,
  which reaches every mapping it held, every registered buffer, every channel
  end, and every capability it ever granted onward; returns what it was made of
  to the `Untyped` that paid for it; and posts a peer-death notice to every
  holder of a channel or an endpoint to it. Its clients see `PEER` refusals and
  a notice, in that order or the other, and nothing else.
- **Restart is a new spawn, not a resurrection.** The manifest declares a
  restart policy; the supervisor applies it; the frame provides only the
  mechanism. A restarted component has a new table, new memory, new channels
  and a higher epoch. The one thing that survives is the *endpoint* — the
  place in the topology its clients hold a capability to — so a client that
  lost its peer reconnects through the handle it already has.

Foreclosed, and named so nobody has to rediscover why: `fork`, `exec` with an
inherited table, signals in any form, `ptrace`-shaped introspection, and
environment inheritance. The rest of this section says each of the five rules
precisely enough to be built.

### Coming into existence: spawn

The manifest is E1-D04's to shape and this RFC's to give meaning to. Four
things in it matter here, whatever the schema calls them.

The **image**, by content hash. The frame copies the bytes into frames it has
just retyped from the supplied `Untyped`, maps text read-only and executable,
and maps nothing writable except what the manifest asks for. The manifest is
reached by the same hash, so what a component *is* — its code and its declared
shape together — is one name that measured boot can extend into. There is no
argument vector and no environment block: configuration is part of the
manifest, which is why it is attestable and why two spawns of one hash are the
same component.

The **needs**: an ordered list of typed slots the component requires to run —
a channel to something, an untyped region of at least so much, an interrupt
vector, an endpoint it may connect to — each with a type, a minimum set of
rights, and whether it is optional. The supervisor's spawn entry supplies one
handle from its own table per need, in the manifest's order. The frame checks
each one: it is of the declared type, it carries at least the declared rights,
and it carries `GRANT`, because handing it on is what the supervisor is doing.
Then the frame **derives** a child of it into the child's table. That word is
load-bearing. The child's capability is a descendant of the supervisor's, so
the supervisor can revoke it later, and so can anyone above the supervisor who
routed the parent down. The derivation tree is the record of grants and the
route for revocation; it is not a channel for inheritance, because the child
receives exactly the listed capabilities and nothing the supervisor happens to
hold. A need not supplied and not optional refuses the spawn. A handle supplied
for a need the manifest does not declare refuses the spawn. Fail closed, R04:
the manifest is a parser, and a parser here refuses what it does not know.

The **account**: an `Untyped` the supervisor supplies, from which the frame
retypes everything the child is made of — its address space root and every
page table, its text and data frames, its stack, the region its control ring
lives in, the region its state tree is published from, and its capability
table itself. This is what gives `Untyped` its object. It is the right to
bring things into existence and the account they are charged to, and a
component's whole footprint is a subtree of one `Untyped` in the derivation
tree — so revoking a supervisor's `Untyped` ends every component it paid for,
and accounting is hierarchical by construction rather than by a second
bookkeeping structure, which is what `deadline-all-the-way-down` section 03
asks for. An `Untyped` that cannot pay refuses the spawn with
`RESOURCE/QUOTA_EXHAUSTED`, and nothing is served from kernel reserve.

The **reservation and domain**, if declared. A manifest that declares a
hard-class reservation is admitted at spawn under RFC 0007 or the spawn is
refused with `ADMISSION`. A component does not start and then discover it
cannot run; R08 says the word deadline is not used for a promise nothing can
refuse, and spawn is the moment of refusal. The manifest's domain field is RFC
0005's, and the frame places the component in its domain at spawn; what a
domain is, and what may share one, that RFC decides.

The spawn completes on the supervisor's control ring with one new handle in the
supervisor's table: an **`Endpoint`** to the child, carrying every right. What
each right means on an endpoint is stated below. The child's first instruction
runs with one register set: the address at which its control ring is mapped.
Everything else it will ever know it learns by draining that ring, and the
first thing it finds there is one *granted* notice per need the manifest
declared, in the manifest's order, each carrying the handle. `door::Entry`'s
first-handle-and-order arrangement, which existed because a component had to
be told rather than left to assume, is retired by this: the component is now
told each one.

### One ring, and the frame at the other end of it

The control ring is the frame's channel of `kernel/src/ring.rs`, one per
component, with the component at the producer end. At E0 both ends of that
channel were the kernel because there was no second address space to map the
region into; this is the decision that module said it was waiting for. The
channel negotiates under RFC 0011 like any other, and it is the one channel on
which `feature::CONTROL_EVENTS` is *required* rather than offered: a control
ring whose peer cannot speak notices is not a control ring, and the spawn does
not proceed.

Two kinds of entry cross it.

**Submissions**, from the component to the frame. The four capability calls of
RFC 0015 arrive here as the opcodes that document said would retire them —
inspect, derive, revoke, map — and four join them: **spawn**, described above;
**connect**, which names an endpoint the submitter holds with `WRITE` and asks
for a channel to whoever occupies it; **stop**, which names an endpoint the
submitter holds with `REVOKE` and a deadline; and **grant**, which names a
capability the submitter holds with `GRANT` and an endpoint, and asks the frame
to derive a child of it into the occupant's table. Every one of them answers in
the error space of RFC 0010 and refuses an unknown flag or a non-zero reserved
field, as every opcode does. Naming the opcodes is the implementer's; their
meanings are this document's.

**Notices**, from the frame to the component. A notice is a completion entry
that answers no submission. It carries a completion flag saying so, its result
is the notice kind, its `user_data` is the handle it concerns, and its `ext`
carries the rest. Seven kinds, and a version of the ABI that adds an eighth
raises `ABI_VERSION` so that RFC 0011 keeps it off a channel whose peer does
not know it; a component that nonetheless meets a kind it cannot name has found
a frame bug and exits saying so, because R04 does not permit it to skip the
entry.

| notice | what it says | `user_data` | what a component owes in return |
| --- | --- | --- | --- |
| granted | a capability was placed in your table, and which need or ask it satisfies | the new handle | nothing; it may now use the handle |
| revoked | a capability you held is gone | the dead handle | stop presenting it; a submission carrying it earns `AUTHORITY/REVOKED` |
| peer gone | the far end of a channel, or the occupant behind an endpoint, ended — and why | the channel or endpoint handle | discard every outstanding token on that channel; reconnect through the endpoint if it still wants the service |
| stop | end yourself by this deadline | the control ring's own handle | quiesce, then `EXIT`; after the deadline the frame ends it anyway |
| reclaim | the core named in `ext` leaves your allocation at this deadline | — | park the work on that core by the deadline; after it the frame preempts mid-task |
| pressure | the grade in `ext` changed for the account that pays for you | — | give something back — a buffer, a cache — or do not, and meet the quota when it bites |
| generation | the system generation is changing, or suspending | — | RFC 0006 and RFC 0012 say what; this RFC only reserves the word |

Three properties of the table matter more than its rows.

*A deadline is monotonic nanoseconds in the control channel's epoch*, exactly
as `Sqe::deadline` is, and RFC 0009 governs it. A stop with no deadline is a
promise nothing can refuse and the frame refuses to make it: a stop submission
whose deadline is `NO_DEADLINE` is an `ARGUMENT` error. A stop whose deadline
has already passed is a kill, and it is spelled the same way as a polite stop
so that the simulator's "kill this driver at a seeded moment" is one opcode
rather than two paths through the frame.

*Reclaim applies to allocations, not to reservations.* A hard-class reservation
holds its cores for its life under RFC 0007 and is never reclaimed; a core in an
ordinary allocation can be. The notice is what makes parking clean possible; it
does not make interruption impossible, and the resource document's rule that
the kernel preempts at allocation boundaries is the deadline in the notice.

*The last four are grades, and the first three are facts about handles.* That
distinction is what the next subsection rests on.

The door shrinks to what RFC 0014 rule 1 permits: `EXIT`, which a component
genuinely cannot submit and then wait on, and the kernel-path doorbell of
E0-B15, which cannot be an opcode on the ring it rings. `ANNOUNCE` is replaced
by the control ring's existence; `PROGRESS` by a blocking wait on it. The
reversal condition RFC 0014 wrote — *the three calls are still here after M5*
— and RFC 0015's — *the four are still here after M5* — fall due at
E1-B05, which is the task that builds the thing they were waiting for.

### Why a notice can neither be lost nor pile up

The frame never waits on a component, so it can never block on a full
completion ring, so the obvious question is what happens to a notice that does
not fit. Two answers were live and both are wrong: dropping it makes the
control ring advisory, and killing a component for a full ring makes a busy
component a dead one.

The answer is that a notice is not a queued event; it is **pending state that
the frame publishes when there is room**. The three handle notices — granted,
revoked, peer gone — are at most one per slot of the component's table at any
moment, so their pending state is two bits *in the slot*, beside the mapping
address that already lives there for the same reason `kernel/src/cap.rs` gives:
a structure beside the table can disagree with it, and a bit in the slot
cannot. The four grades are at most one per kind per component, latest wins:
a pressure grade that changes twice before the component drains once is one
notice carrying the second grade, because the first was never true of anything
the component could still act on. The frame posts pending state in a fixed
order — slots ascending, then the grades in the order of the table above —
whenever the completion ring has room, so the ring's depth bounds how much is
*visible* and never how much is *true*.

Two consequences are stated so they are not discovered. A capability granted
and revoked before its granted notice was ever drained posts nothing: a
component that was never told it held something never held it, and the frame
clears both bits. And ordering across kinds is not promised — a component that
drains late sees a revoked notice for slot three before a peer-gone for slot
nine whatever the order the events had — while ordering within a slot is: no
slot posts revoked before granted. A component that needs to know *when* reads
`Cqe.timestamp`, which every completion already carries.

This is also why the pending state belongs to E1-B13 rather than to the ring
code. The table becomes an object paid from `Untyped` in that task, and the
notice bits are part of what a slot costs. A component's whole notice surface
is then bounded by what it has paid for, which is the same bound everything
else in this design has.

### Authority: routed, or asked for through a broker the user sees

Routing is the spawn above: the topology of `deadline-all-the-way-down` section
07, evaluated by a supervisor into a manifest and a list of handles, so *what
can this component reach* is computed from the configuration before it runs.
This RFC adds the run-time half the design corpus named and did not build.

A manifest may declare **asks** beside its needs: capabilities it does not
receive at spawn and may request while running — the file the user will pick,
the device the user will plug in. An ask is a typed slot like a need, with the
one difference that nobody supplies it at spawn. To have one satisfied, the
component connects to the **powerbox** — a component like any other, whose
endpoint reached it through its manifest's needs like any other — and submits
a request naming the ask by index. The powerbox holds authority over a
namespace the requester does not: a store, a device tree, a directory of
peers. It resolves the request — in E1 against a policy it was routed, because
there is no compositor yet and the user is a file; at E3, by putting a picker
in front of a person — and then submits a **grant** on its own control ring
naming the object and the requester's endpoint. The frame derives a child of
the powerbox's capability into the requester's table and posts a granted notice
carrying the ask's index. The requester receives a capability to the object
the user named and never holds authority over the namespace it came from. That
sentence is the whole of the confused-deputy answer, and the reason the shape
is decided now is so that the compositor implements a picker, not an authority
model.

A grant is a derivation, and this RFC says so knowing what it costs. The
capability in the requester's table is a child of the powerbox's slot; when the
powerbox instance dies, its table is revoked, and every grant it ever made is
revoked with it — each requester sees a revoked notice and asks again through
the refilled endpoint. `kernel/src/cap.rs` and RFC 0015 both name *a broker
holding capabilities on behalf of others* as the case where the copy-is-a-child
rule might be the wrong default. It is the case here, and this RFC chooses
uniformity over it deliberately: one derivation rule, one revocation walk, and
a broker restart that is *visible* as a burst of revoked notices rather than
silent as a set of grants nobody can find the root of. The reversal condition
is below, and it is measurable.

`rights::GRANT` is read as *may be granted onward*: the frame may derive a child
of this capability into another component's table. It is not a move. `abi/src/
cap.rs` says "transferred", and this is the reading of that word: the source
keeps its handle and its place in the tree, and the recipient's is a child.

**`Endpoint` acquires its object.** An endpoint names a *place in the
topology* — one manifest's slot under one supervisor — and, through it,
whichever instance currently occupies that place. The six rights read on it as
follows, and the bitmap is not widened to say so:

| right | on an endpoint |
| --- | --- |
| `READ` | may map the occupant's published state tree, read-only — RFC 0013's mapping, reached through the map opcode with the endpoint as its operand |
| `WRITE` | may connect: submit *connect* and receive a channel to the occupant |
| `EXECUTE` | undefined; a derivation asking for it is refused |
| `DERIVE` | may mint a weaker endpoint — connect-only, say — to route to a peer |
| `REVOKE` | may stop the occupant, and may refill the place by spawning into it |
| `GRANT` | may hand the endpoint to another component |

The supervisor holds all six. A client is routed `WRITE`, and usually `GRANT`
so it can route onward; a monitor is routed `READ`. Nothing holds `REVOKE` on
an endpoint except the component that spawned into it and whoever that
component chose to derive it to — which is what *a supervisor* means here, and
it is a role a capability confers rather than a kind of component.

*Connect* on an endpoint whose place is empty does not fail; it pends, with
the submitter's deadline, until a spawn refills the place or the place is
retired. That is the mechanism behind gate G1's sentence — a driver is killed
under load and the system does not notice — and it is stated here so E1-P06
tests a design rather than a coincidence.

### Ending, and what everyone else sees

Three ways in and one way out, as `process.rs` already says of its smaller
subject. A **fault** at ring 3 — any exception, and also a control ring whose
header the component corrupted, which the frame treats as the component having
stopped speaking. An **exit**, through the one door call, with a status. A
**stop** whose deadline passed with the component still running, or arrived
already past. In all three the frame does the same things in the same order,
and the order is fixed because a seeded run has to reproduce it:

1. **Revoke the table.** Every slot, in slot order, as `Table::clear_all` does
   now, but with the walk crossing tables: every descendant in every other
   component's table is revoked and its holder posted a revoked notice; every
   mapping a revoked capability authorised is unmapped with shootdown, as
   E0-B10 made revocation do; every registered buffer set — an object under
   E1-D03, and so a capability — is revoked, which tears down the component's
   IOMMU domain under E1-B01, so that a device transfer still in flight to a
   dead component's memory faults rather than lands. A capability the
   component *granted onward* is a descendant and dies here too; that is the
   powerbox cost above, and it is the same rule, not a second one.
2. **Tear down its channels.** Both mappings of each channel region go; the far
   end's next submission on that channel earns `PEER/GONE`, and its holder is
   posted a peer-gone notice with the channel's handle. The two are not
   ordered against each other — one is a refusal at the moment of use, the
   other a notice at the next drain — and a client handles either arriving
   first.
3. **Post peer-gone to every holder of an endpoint** to the place it occupied,
   with the cause in `ext`: the fault vector, the exit status, or that a stop
   deadline passed. The supervisor is one such holder, and this is how it
   learns; there is no separate wait-for-child.
4. **Return the memory** to the `Untyped` it was retyped from. What an account
   paid for comes back to that account, not to a global free list, which is
   what makes a supervisor's quota a real number after its children have lived
   and died.
5. **Record it.** The frame's own state tree counts faults, exits and stops per
   place, which is where the blast-radius claim E1-P06 gates on gets its
   numbers from — read, under RFC 0013, never delivered.

The component's own state tree is gone with its memory. A supervisor that wants
a component's last words reads the tree *before* the stop deadline, which is
what a deadline is for; there are no last words after a fault, and the frame
does not pretend otherwise by keeping a copy.

### What restart means

The manifest declares a **restart policy**, and E1-D04 owns its spelling. What
this RFC fixes is its semantics, whatever the spelling:

- Restart is the **supervisor's** act, not the frame's. On a peer-gone notice
  for an endpoint it holds with `REVOKE`, the supervisor consults the policy
  and, if the policy says so, submits spawn *naming that endpoint*. The frame
  refuses a spawn into an occupied place, and refuses a spawn into a place
  with a different manifest hash from the one that created it — a different
  manifest is a different place, and E2-D04's state-transfer protocol is where
  a newer manifest may lawfully take over an older one's place. Policy lives in
  a component and mechanism in the frame, which is the division everything
  else in this system makes.
- The policy distinguishes at least *never*, *on fault* and *always*, and
  carries a **budget**: how many restarts in what window before the supervisor
  stops trying. The window is read from `Env`, so under the simulator a
  restart storm is a seeded scenario and not a wall-clock accident. A place
  whose budget is exhausted is **retired**: its endpoint is revoked in every
  holder's table, pending connects complete `PEER/GONE`, and the supervisor's
  own supervisor is told by the ordinary route — a revoked notice for the
  endpoint it routed down. This is `deadline-all-the-way-down` section 07's
  *a driver that fails to start leaves its subtree unstarted*, extended past
  boot.
- **Nothing carries over.** The new instance is spawned from the same manifest
  with the same needs, each re-derived from whatever the supervisor holds *at
  that moment* — so a supervisor that lost a capability in between cannot
  route it, and the respawn refuses the way a first spawn would. New table,
  new memory, new channels. State transfer between instances is E2-D04's
  protocol and a manifest that cannot do it says so there.
- **Epoch.** A channel opened to the occupant of a place carries, in its
  header's `epoch` field, the ordinal of that occupant: the first instance
  opens at zero, the first restart at one. `ring-scene-boot` section 06 says
  a restarted peer increments the epoch and a mismatch means every outstanding
  token is stale; under this RFC the region does not survive the peer, so the
  field's job is to tell a reconnecting client, in the first cache line of its
  new channel, that this is not the peer it had.

### What is foreclosed

Written as mechanisms that are unavailable, per R01, rather than as things we
intend not to do.

- **`fork`.** No operation creates an address space from the contents of
  another. The only way to a new address space is a spawn, and a spawn's
  address space is retyped from `Untyped` and filled from an image by hash.
- **`exec` with an inherited table.** No table survives any boundary. A spawn's
  table is filled from a list the frame checked item by item; there is no
  operation that copies a table, and no flag that says "and everything else".
- **Signals.** A component has one path in: the completion ring of its control
  channel, drained when the component chooses to drain it. There is no handler
  registration — `cargo xtask lint-callbacks` already fails a build that
  registers one — and therefore no interrupted instruction stream, no masking,
  no `EINTR`, and no concept of async-signal-safety anywhere in the system.
- **`ptrace` and its relatives.** No capability type and no opcode reads or
  writes another component's memory or registers, and no right on an endpoint
  will be given that meaning. A supervisor sees its child through its state
  tree and its death notice. A debugger is a component substitution in the
  simulator of E1-P01, where every byte is already visible, rather than a
  peephole in the frame that a compromised supervisor would also have.
- **Environment inheritance.** There is no environment. Configuration reaches a
  component in its manifest, by hash, and a component that needs to behave
  differently is a different manifest.
- **Asking the frame by name.** There is no namespace the frame resolves. A
  component that wants something it was not routed asks a broker it *was*
  routed, and the broker's answer is a derivation the broker can withdraw.

### What each task builds from this

Named so that each can be built without re-deciding.

**E1-B05, the supervisor.** The first long-lived component that is not `init`,
and the first that holds an `Untyped` it did not consume itself. It parses
E1-D04's manifests and refuses what it does not know; evaluates a topology
into spawn entries; holds an endpoint with all six rights per place; applies
the restart policy on peer-gone notices with the budget window read from
`Env`; and refills places by spawning into them. On the frame side, this task
adds the spawn, connect, stop and grant opcodes to the frame's channel, maps
one such channel per component, posts the seven notices from the pending state
E1-B13 provides, and retires `ANNOUNCE`, `PROGRESS` and the four capability
calls from the door, leaving `EXIT` and the doorbell. E1-P06 is its exit: a
driver killed under load, a client that reconnects through the endpoint it
holds, and a blast-radius number read from the frame's tree.

**E1-B13, the table as an object.** The table is created at spawn, paid from
the supplied `Untyped`, sized as the manifest declares, and grown on request
from the component's own `Untyped`; a component that cannot pay is refused
`RESOURCE/QUOTA_EXHAUSTED`. The parent link learns to name a slot in another
table, so the derivation tree spans components and the revocation walk crosses
them — still iterative, still bounded, now by the slots in existence rather
than by one array, which is bounded by what `Untyped` has paid for. Each slot
carries the two pending-notice bits, and each table carries the four grade
words. The five properties and the negative suite hold at every size, and a
process that could not previously hold more than the fixed count now can. This
is the task the fixed table said it would break on, and it does.

**E1-B08, the user-level runtime.** Drains its control ring at every polling
point it already has, and never installs anything that looks like a handler.
On *reclaim* it parks the work on the named core before the deadline and
reports, through its state tree, how often it did not. On *stop* it stops
submitting, drains its own rings to a quiescent point, and calls `EXIT` before
the deadline; the frame's count of stops that became kills is the number that
says whether the deadline the supervisor chose was honest. On *pressure* it
releases what it can and does not have to. Its exit — zero kernel entries on
the hot path — is unaffected by any of this, because a drain is a load from a
mapping and not a call.

## Context

What was true when this was decided. One process runs at a time, on a core that
is not the one that built it, and is ended by the frame counting ticks
(`kernel/src/process.rs`). A process holds a fixed table of thirty-two
capabilities in a per-core static, and `kernel/src/cap.rs` names E1's first
supervisor as the thing that breaks that. Seven calls stand behind the door,
two of which RFC 0014 promised would retire at M5 and four of which RFC 0015
did; M5 arrived at E0-B12 and none retired, because the ring it landed had the
kernel at both ends. `Endpoint` and `rights::GRANT` exist in `abi/src/cap.rs`
with comments pointing at this document. `feature::CONTROL_EVENTS` exists with
a one-line description and nothing behind it. And
`docs/what-must-be-stated.html` lists `fork()` and *Signals* (section 04) and
*Pressure is discovered, not delivered* (section 08) as open — the same hole
seen three ways: a component has no path by which it is told anything — and
sketches this RFC in one paragraph of section 16, which this document expands
rather than contradicts.

The design corpus describes an assembler instantiating a declared topology at
boot (`deadline-all-the-way-down` section 07) and says a driver "carries a
declared restart-on-crash policy" (`fast-path`, after Fuchsia's second driver
framework). Neither says how a component comes to exist after boot, how it
learns its world changed, or what the powerbox the architecture's bet 04 leans
on actually does. Genode's recursive composition — every component is a parent
that pays for and can destroy its children — is the model for the `Untyped`
account; Fuchsia's declarative routing is the model for needs and the
topology; seL4's `Untyped` retype is the mechanism under both. What none of
them settles for this system is the notice path, because all three permit
some form of asynchronous delivery and this system's determinism contract does
not.

Why now, and not at E0 or E2: `TODO.md` says it in the exit line. E1-B05 is the
second long-lived component. Its lifecycle is either designed before it exists
or retrofitted after, and a lifecycle retrofitted onto one component is a
special case that the third component then has to match.

The alternatives that were live, and why each lost:

- **`posix_spawn` with an explicit list, and an environment.** The list is
  right; the environment is inheritance with a smaller surface, and R06 does
  not say "less". Rejected.
- **`fork` and `exec`.** `what-must-be-stated` section 04 says a capability
  system cannot use it and section 13 admits what is lost: it composes
  beautifully at a prompt. Rejected, and section 16's own reversal line for
  this RFC reads *nothing plausible*.
- **A small set of asynchronous notifications for the "urgent" cases** — kill
  and reclaim — with everything else on a ring. Rejected because reclaim is
  exactly the case that needs a deadline rather than an interrupt: the
  resource document's whole point is that a runtime parks *cleanly*, and an
  interrupt is the mechanism that makes cleanly impossible. Kill needs no
  notice at all. There is no urgent case left.
- **One ring per kind of notice.** Rejected: N rings to drain at every polling
  point, N depths to size, and an ordering question between them that one ring
  answers by construction.
- **The supervisor holds a direct channel to its child** and sends stop and
  reclaim itself. Rejected because then a hostile or hung child can hold its
  supervisor's channel, and because notice semantics would be a convention
  between two components rather than a property of the frame. The frame is
  the only producer on every control ring; a supervisor asks it.
- **Notices as completions of a standing subscription entry**, `io_uring`
  multishot style, so that every completion answers a submission. Attractive
  for its uniformity and rejected for one reason: the initial granted notices
  must be in the ring before the component's first instruction, and there is
  no submission yet for them to answer. The flag on the completion is the
  smaller change.
- **An endpoint names an instance.** Simpler to implement and rejected because
  it makes every client of a restarted driver need a fresh grant from a
  supervisor that must therefore know every client. Gate G1 needs a client to
  reconnect through what it already holds.
- **The frame restarts components itself**, from the policy in the manifest.
  Rejected because policy in the frame is policy nobody can replace, and
  because a restart needs capabilities re-derived from the supervisor's
  *current* holdings, which only the supervisor can present.
- **Grants that survive their grantor** — the broker case. Deferred, not
  rejected, and the reversal condition below is where it comes back.

## Consequences

**Easy.** A second component, and a third, and an imported driver, are all the
same thing: a manifest, a supervisor, a control ring. E1-P06 has a design to
test rather than a behaviour to observe. The simulator gets component death and
restart as two opcodes with seeded deadlines. `Untyped` accounting is a
subtree, so a quota is a real number and an out-of-memory decision is local to
the account that overspent — there is no global victim selection, which RFC
0004 needs to stay true. Every notice is a completion entry, so the hostile-peer
fuzzer of E1-P04 already covers the control ring's wire without a second
harness. And the door finally shrinks to the two calls RFC 0014 could defend.

**Hard.** The pending-notice bits make the capability table carry protocol
state, and E1-B13 has to keep them honest across grow, revoke and death in the
same walk. Cross-table parent links make the revocation walk's bound a system
property rather than a constant, and the argument that it is still bounded
rests on `Untyped` accounting being airtight. A supervisor's `Untyped` revoked
ends every component under it at once, which is correct and which somebody
will do by accident. Endpoints as places mean a client can be connected to an
instance that has never seen it, so every protocol above a channel must treat
epoch zero and epoch one identically except for discarding tokens — which the
ring rules already require and nobody has yet had to obey.

**The honest cost.** A broker's restart revokes everything it granted. In E1
the broker is a policy and the cost is a burst of revoked notices and re-asks;
at E3 the broker is a picker and the cost may be a person asked twice. That is
recorded here as a cost, per R12, rather than hidden inside the uniformity it
buys.

**Forecloses.** Everything in the section of that name, plus one more: a
component that outlives the account that paid for it. There is no way to spawn
without an `Untyped`, and no way to keep a child alive when its `Untyped` is
withdrawn. That is the property that makes "kill the supervisor" a bounded
event in the simulator rather than an orphan hunt.

## What would reverse this

**Polling costs more than the delivery it replaces.** The condition
`what-must-be-stated` section 16 wrote for this RFC before it existed. The
measurement is E1-P10's kernel-entries-per-operation claim with control-ring
drains counted separately: if draining the control ring at every polling point
shows up on the hot path, the answer is not signals — it is the suppression and
user-interrupt doorbell of E1-B09 applied to the control ring, and this RFC's
rule that the control ring is a ring like any other is what makes that answer
available.

**A broker restart is unacceptable in practice.** If E1-P06's chaos, applied
to the powerbox, produces re-asks that a client cannot absorb — or if at E3 a
person is asked twice for the same file often enough to notice — then a grant
needs to survive its grantor, and `rights::GRANT` acquires a second reading:
re-parent onto the grantor's own parent at the moment of grant. RFC 0015 wrote
that reversal as *a copy needs to survive its source being revoked*; this RFC
makes it observable and puts a workload behind it.

**A place is the wrong thing for an endpoint to name.** If E2-D04 finds a
class of component whose clients must never be silently connected to a
successor — because the successor cannot carry the state the client believes
it shares — then those clients need an endpoint that names an instance and
dies with it. That would be a second endpoint flavour rather than a reversal
of the first, and the manifest is where a component would declare which it
offers.

**A runtime cannot park inside any deadline the supervisor can name.** Then
reclaim degenerates into preemption, and the resource document's user-level
runtime model is what is wrong, not the notice. The frame's count of reclaims
that became preemptions is the number, and a stop-to-kill count is its twin.

**Introspection the state tree cannot provide.** A driver wedged on hardware
the simulator does not model, with no way to see why. The answer would be a
manifest-declared debug endpoint — authority a component *grants* to its own
inspector at spawn — and not a frame-level peephole; it would amend the
foreclosed list rather than delete it, and it should be written as an RFC that
says what the state tree could not show.

**A component that legitimately has two paths in.** None is known. If one
appears — a device whose hardware delivers something a ring cannot carry —
it is the reversal of R05 itself, and it belongs in an RFC about that rule
rather than in an amendment to this one.
