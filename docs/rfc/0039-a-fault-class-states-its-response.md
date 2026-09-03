# RFC 0039: A fault class states its response before it is injected

- Status: accepted
- Date: 2026-09-03
- Affects: `sim/` (new: `fault.rs` — `Class`, `Injection`, `Injector`, and the
  seven assertions; `lib.rs` — `World::arm`, `World::strike`,
  `Outcome::injected`; `scenario.rs` — a `injects` field and seven scenarios;
  `dev.rs` — `Injured`, `Bus::new` and `Protocol::HONOURS`; `blk.rs`, `net.rs`,
  `gpu.rs` — a `HONOURS` declaration each; `client.rs`, `service.rs` — one
  consultation each), `docs/test-taxonomy.md` and its TOML twin (three rows).
  Executes the fault-class half of RFC 0032 and spends the domain word
  `decide.rs` reserved. Reverses nothing.

## Decision

**A fault class is a site, a scenario and an assertion, and the assertion is
written before the injection.** The class states in advance what the system must
do — *this registration is refused with `RESOURCE`/`QUOTA_EXHAUSTED` and the
component issues nothing after it*, *this buffer comes home and no completion
arrives after the reset*, *this run finishes later and nothing else moves* — and
the model is then made to do it. Three consequences follow, and each is the part
somebody would otherwise do differently:

- **A class has one fault kind, not a drawn one.** `f_env::sim::SimEnv` draws
  `Fail`, `Delay` or `PeerRestart` from the site's own stream, and that is right
  for a sweep hunting anywhere. It is wrong for a class, because a class whose
  *allocation failure* were sometimes a delay would have no response to state.
- **A fault plan belongs to the scenario, not to the command line.** What is
  broken and how often is a field of `Scenario`, so a reproduction stays
  `f-sim --trace --seed 0x… <name>`.
- **Fault draws are keyed at `decide::domain::FAULTS`,** the word E1-P01 spent in
  advance, and never through a second `Env`.

## Context

E1-P02's exit criterion is *each class has a scenario, and each scenario has a
system response that is asserted rather than observed*, and the interesting word
is the last one. The apparatus already had everything needed to inject: a hook
with per-site independent streams (`env/src/sim.rs`, M0), a splittable generator
(RFC 0026), a simulator with device models and a hashed artefact (RFC 0032,
0034, 0035), and a reserved domain word. What it did not have was a rule about
what an injection is *for*.

The live alternative was the obvious one, and it is what most fault harnesses
are: arm a rate, run a sweep, and let the run report what happened. It costs
nothing to build and it decays in a predictable way — a scenario that prints its
outcome needs a reader, the reader is whoever is on call, and within two quarters
nobody reads it. The output is then indistinguishable from coverage while
proving nothing, which is the one failure mode a test apparatus must not have and
the same one `sim_check`'s second seed exists to prevent one level down.

Three smaller alternatives were live and are recorded because each looks better
than it is.

**Drawing the kind, as `SimEnv` does.** It gives one injector for every class and
more trajectories per seed. It also makes the class a *distribution over
responses*, and there is no sentence of the form "the system must do X" to write
about a distribution. `SimEnv` keeps its behaviour: the two are aimed at
different things, and `Fault` is re-exported rather than redefined so that they
remain the same three words.

**Putting the plan on the command line.** It would let a sweep vary injection
without touching the table. It would also make `(seed, commit)` an incomplete
bug report — a failing run would need a seed *and* an argv — which is precisely
what E1-P03 has to be able to print in one line and E1-P08 has to re-enter.

**Injecting through `SimEnv` itself.** `World` already *is* an `Env`, with the
timeline's clock and a stream split off the run's seed. A `SimEnv` inside it
would be a second clock and a second generator in one run, and a reproduction
check cannot be taken over two sources of time.

## Consequences

**Seven classes, seven scenarios, seven assertions, and a table that says so.**
`every_class_has_a_scenario` holds the enum against `SCENARIOS`, and
`every_armed_scenario_actually_strikes_and_writes_it_down` requires each armed
class to fire at three seeds — so a class that stops being reachable is a red
suite rather than a quiet one. That is what moves the taxonomy's *a
fault-injection site that is never exercised* row from *partially* to *catches*.

**Injection is visible in the hashed bytes.** A strike writes a record — the
class, the operation's token, and which consultation of that class it was — and
the artefact's header names what was armed. An artefact produced under injection
that did not say so would be quoted later as a clean run; this is the same
argument `dev.rs` makes for writing a dropped completion down rather than
staying silent, and it is what E1-P03 reads to report a failure.

**The four latency and protocol classes are asserted at the client, not at the
model.** *No client observes anything except added latency* is E1-P06's exit
sentence, and `faultin` and `latecqe` assert it here against a baseline run of
the same scenario with nothing armed — which is the only honest comparison,
because a threshold in nanoseconds would be a number that needs a claim.

**Adding the eighth class costs nothing that already exists.** A class's answer
is a pure function of the seed, its own label and its own consultation count, so
consulting a new class on a path a recorded seed never enters moves neither that
seed's faults nor its interleaving.
`arming_a_class_that_never_fires_leaves_every_scenario_exactly_as_it_was` checks
it over the shipped table and over a scenario that already injects, which is the
shape in which the property is easiest to lose.

**Two classes reach the models as things a device cannot detect.** A translation
the domain declines and a write of the device's own that does not land are not
expressible as refusals a protocol could return, because from the device's side
neither has happened. They arrive on `dev::Bus` as `Injured`, and `Bus::granted`
and `Bus::writes_land` are the only doors — so a protocol asks *may I* rather
than *is a fault armed*, and a device model never learns it is being tested.

**And a protocol declares which of the two it reads.** `Protocol::HONOURS` lists
the bus classes a device model actually asks about, and `Device::poll` consults
exactly those. It is a check and not a tidy-up. `Class::Partial` reaches a
protocol only through `Bus::writes_land`, and today only `blk` asks — a network
interface writes nothing back into control memory, which is a protocol fact
rather than an omission. Ungated, arming `Partial` against `net` would *strike*:
the class would be consulted, drawn, and written into the hashed artefact, and
the run would be identical in every other respect. That is a site consulted and
not exercised, passing for coverage — the third shape of
`docs/test-taxonomy.md`'s *a fault-injection site that is never exercised*, and
the one the other two checks cannot see, because the class does have a scenario
and the scenario does strike. Gated, it never strikes, and
`every_armed_scenario_actually_strikes_and_writes_it_down` turns it red.
`a_class_a_protocol_does_not_read_is_never_consulted_there` asserts both halves,
so the gate cannot quietly become a no-op. The classes the machinery injects — an
allocation refused at the domain, a page fault added to the service time, a peer
that stops, a torn doorbell — are not listed, because every device honours them
by construction and a list that named them would be a list nobody could get
wrong.

**A fault plan has one monotone handle and it is not the obvious one.** E1-P03
minimises, and the shape of the minimiser is decided here rather than found
there. Dropping a whole `Injection` from a plan is monotone and asserted.
Neither number is. `Injection::one_in` is a modulus, so raising it selects a
*different* set of occurrences and can drop the very strike that failed.
`Injection::after` is subtler and the one a minimiser would reach for first: it
leaves a class's draws exactly where they were, because `draw` is keyed by the
occurrence and nothing else — but a strike **changes the run that produces the
consultations**, so which operation occurrence *k* names moves with it.
`Class::PeerGone` makes the point without an experiment: `Device::service`
returns immediately once the device has reset, so the device is consulted exactly
once past `after`, and raising `after` removes nothing at all — it relocates the
single strike onto a different operation. A minimiser that raised it, still saw a
failure, and concluded the earlier strikes were not required would have lost the
bug and kept a green-looking reproduction. Both doc comments say so at the
fields, because that is where somebody writing the minimiser will be looking.

**What is foreclosed, and what is merely not built.** Foreclosed: a class with no
stated response, which is now a compile-time absence rather than a judgement.
Not built, and named rather than left as a silence:

- **The other tear of a doorbell** — an entry published with no bell. This
  model's device takes one entry per doorbell, so a lost bell would be a lost
  entry rather than a late one, and what that exercises is the model's shape.
  A peer that lies about its cursors is E1-P04's.
- **A partial write reported as a short used length.** `Blk::harvest` reads the
  status byte and not the length, deliberately and for the reason `dma.rs`
  records — so a fault aimed at the length would pass with the client unable to
  tell anything had happened. The class tears the status byte instead, which is
  what `STATUS_NONE` was written for and what nothing previously reached.
- **The classes `proving-ground.html` layer 1 lists that E1-P02 does not name** —
  bit rot, misdirected writes, clock drift, thermal transitions. The design
  document is ahead of the code by design; these are a `TODO.md` line each, and
  each would arrive as the same three things.

## What would reverse this

**A class whose honest response is a distribution.** If a fault turns out to
have two legitimate system responses that a scenario cannot separate — the same
injection correctly answered one way under one interleaving and another way under
another — then *one kind per class* is buying a false crispness, and the class
should draw its kind and assert a predicate over the set of responses instead of
an equality. Concretely: a class whose assertion has to be written as *either
this or that* for reasons that are not a scenario's to fix.

**A sweep that needs to vary injection faster than the table can be edited.**
E1-P03 runs seeds nightly. If it turns out that finding bugs needs the *plan*
swept as well as the seed, the plan has to become part of the run's identity —
which means a reproduction quotes `(scenario, plan, seed, commit)` and the plan
is derived from the seed rather than written in the table. The test that this has
happened is E1-P03 reporting a failure it cannot reproduce from a scenario name
and a seed.

**A `HONOURS` list that is wrong more often than the hole it closes.** The
declaration is only worth its weight while forgetting to add a class to it is
louder than forgetting to read one off the bus — and today it is, because a
missing entry makes an armed scenario strike zero times and fail a test, while a
wrong entry makes a class strike and change nothing, which is the thing being
prevented. The signal that it has inverted is a review finding of the form *the
class was in `HONOURS` and the protocol stopped reading it*, which no test here
would catch. The replacement is a `Bus` that records which doors a `serve` call
opened and a check that every armed bus class had its door opened at least once
in the run — strictly better, and not built now because it is machinery the
seven classes do not need.

**A frame event whose client-visible refusal is not the interesting half.** RFC
0032 put allocation failure and translation fault here on the argument that the
response is the component's. If a defect is found that lives in the frame's
behaviour *during* one of those refusals — a half-registration, a translation
left behind — and the boot suite and the bounded proofs cannot reach it, then the
class belongs where the allocator is and not where the client is.
