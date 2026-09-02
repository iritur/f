# RFC 0026: One derivation, and streams that split by identity

- Status: accepted
- Date: 2026-09-03
- Affects: `env/`, RFC 0004, `docs/design/proving-ground.html` layer 1, E1-P01,
  E1-P03

## Decision

`env/` has one derivation and one generator, in `env/src/split.rs`.

- `mix` is SplitMix64's finaliser, and it is the only mixing function in the
  crate.
- `derive(parent, identity)` keys a seed by an identity, mixing the identity
  before it meets the parent. It is injective in each argument and asymmetric.
- `Stream` is SFC64: three chaining words and a counter, seeded through a
  SplitMix64 sequence with the twelve-step warm-up its author specifies.
- `Stream::split(&self, identity)` derives a child **by identity and not by
  drawing from the parent**. It takes `&self`, advances nothing, and always
  names the same child for the same identity.

`SeededEnv` holds one `Stream`. `sim.rs`'s per-site draw is
`derive(derive(seed, label(site)), occurrence)` — the same derivation, applied
twice, keyed by the site and then by that site's own occurrence count.

The generator is documented with its period floor (2^64 outputs, guaranteed by
the counter), the reason it has no all-zero fixed point, and its three known
weaknesses. No floating point appears anywhere in `env/`, including its tests.

## Context

There were two generators. `SeededEnv` ran xorshift64 with a guard against the
all-zero state; `sim.rs` hashed a site label with FNV-1a, rotated it, xored it
into the seed alongside a counter, and finished with SplitMix64's finaliser.
Both said in a comment that they were chosen for reproducibility and not for
statistical quality, and both were right to: with one seed and one stream,
reproducibility is the whole requirement and quality buys nothing.

That stops being true at E1-P01. The simulator multiplies streams — a stream per
site, per device model, per component — and E1-P03 sweeps thousands of seeds
across them nightly. **A sweep whose streams are secretly correlated explores
less than it reports, and reports the smaller number as the larger one.** That
is the one failure mode a test apparatus must not have, because every other
layer of the apparatus reads its output as coverage.

Two alternatives were live.

*Keep both generators and test them.* Rejected because the interesting property
is a relation between streams, and two constructions have to be shown
independent of each other as well as internally sound. One derivation makes that
question disappear rather than answering it.

*Split by drawing a seed from the parent*, which is what most libraries do.
Rejected for the reason `sim.rs` already had written down at length: a child
seeded from the parent's output depends on how many values the parent had
produced, so adding a consumer anywhere shifts every stream created after it,
and a recorded seed silently stops reproducing. Splitting by identity is the
same argument the per-site design already rests on, generalised.

`E1-B11` is placed before `E1-P01` and not after because a seed corpus is priced
in the generator it was drawn from. Migrating one afterwards means either
re-running the sweep that produced it or keeping a corpus whose seeds mean
something the tree no longer computes.

## Consequences

**Every seed means something different at this commit.** `(seed, commit_hash)`
was always the contract, so nothing recorded breaks: a seed reproduces the run it
was recorded against, and that run was recorded against a commit.

The kernel's boot log moves on nine lines, and it is worth separating them,
because only one of the three groups is about the generator.

- `env digest`, which is eight draws from `SeededEnv`. This is the line the
  change is for.
- `user space ... root`, because `kernel/src/mem.rs`'s frame-allocator self-test
  frees its frames in an order `env.scheduler().choose()` picks. A different
  generator gives a different free order and therefore a different address for
  the next allocation. That is the substrate doing its job: the order is
  adversarial *and* a pure function of the seed, which is why it moved at all.
- `module`, `frames`, `address space`, `frame hygiene`, `state 3 total`,
  `state 4 free` and the state-tree snapshot that hashes the last two. None of
  these is about randomness. The debug image's last loaded segment ends at
  `0x1EB000` before this change and `0x1EC000` after it — one 4 KiB page of new
  code — so the boot module is placed one frame higher and one frame leaves the
  usable map. Any commit that adds code to the kernel moves these lines, and the
  boot log carries the image's extent on purpose.

`cargo xtask trace` still reports one hash for two runs of this commit, which is
the property that matters and the one E0-P02 gates on.

`docs/first-boot-outside-qemu.md` records a boot log containing the old digest.
It is a dated record of a run at a commit and is left alone, because editing a
record to match today is how a record stops being one.

**`SeededEnv` grew from one word of state to five.** Forty bytes instead of
eight, in a struct the kernel holds one of. That is the price of a generator with
a stated period.

**Seeding costs twelve steps.** `SeededEnv::new` and every `split` run the
warm-up. It is about a hundred instructions, once, and it is why `sim.rs`'s
per-site draw is two `derive` calls rather than a `Stream` per site: a
counter-keyed draw is constant-time random access, and a stream would have to be
stepped to its occurrence or stored in a table that has sixteen slots and no
allocator behind it.

**The comment this RFC exists to delete is deleted.** "Chosen for
reproducibility, not for statistical quality" is gone from both places, and what
replaces it is a period floor, a fixed-point argument, and three weaknesses named
out loud — including that the generator is not cryptographic and must not be
reached for when M4 needs unpredictability rather than reproducibility.

**The cross-stream bound is a test, with its band derived rather than fitted.**
Bit agreement over the low 32 bits of paired draws is a binomial with a known
variance, so the band is five standard deviations computed from the number of
draws. The test that watches it reject something — two copies of one stream —
ships beside it, because a statistic nobody has seen reject anything is
indistinguishable from one that cannot. What the statistic does not detect is
written down next to it: correlation between bit positions, structure across
successive outputs, the high 32 bits, and a stream that is another one at a
different phase. The last of those has a test of its own for that reason.

## What would reverse this

A statistical failure the sweep finds — SFC64 is empirically tested rather than
proved, and there is no equidistribution result for it. Or a consumer that needs
unpredictability against an adversary rather than reproducibility, which is
capability tokens and address-space layout at M4: that is a hardware source and a
real construction behind the same `Env` method, not a change to this one.

Either is a different generator and therefore a different commit, which is the
whole of the migration story and the reason it is cheap.
