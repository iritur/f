# RFC 0110: A demonstration is published so that a corpus can refuse it

- Status: accepted
- Date: 2026-09-24
- Affects: `interface/src/token.rs`, whose seven themes move out of its test
  module and become `SHIPPED`; `claims/0035-theme-refusals.toml`, whose
  `[workload]` note said in prose what this makes arithmetic;
  `claims/theme-corpus/`, the corpus this exists to let a run refuse;
  `xtask/src/main.rs`'s `theme_report`, which is the caller; RFC 0079, whose
  demonstration this publishes without changing any of its decisions

## Decision

The seven themes `interface/src/token.rs` carries — `Theme::DEFAULT`, `DARK`,
and the five hostile ones — leave `#[cfg(test)]` and become a public constant,
`SHIPPED`, a table of `(name, Theme)` pairs. The test module reaches them through
`use super::SHIPPED as THEMES` rather than declaring its own list, so there is
exactly one of them and a theme added to the argument is a theme every reader of
the table knows about.

**The reason is a refusal that could not otherwise be arithmetic.**
`claims/0035-theme-refusals.toml` publishes a share of themes that survive this
layer untouched, over a corpus whose every theme was written by somebody not
working on that module. Its `[workload]` note already said what the seven are —
*a demonstration rather than a sample; a share computed over them would be a share
computed over the answers* — and that sentence was the whole of the protection. A
guard that exists only as prose in a claim file is a guard that holds until
somebody in a hurry needs a corpus, and this repository has twice recorded what
happens to a check nobody can run: `claims/0017` spent a whole task green over a
bench its own route did not run, and `claims/0018` was audited for the same shape.

So `cargo xtask claim theme-refusals` compares every corpus entry against all
seven **by value** and refuses the whole corpus when one matches. By value, because
a name is something a corpus entry writes and a value is not: an entry that
transcribed `hostile-flat` and called it *some-desktop 4.2* is exactly the entry
the check exists to catch, and an entry that merely calls itself something is not
accused of anything.

**One matching entry refuses the corpus rather than being dropped from it**, and
that is a separate decision from the publication. A non-independent entry is
*excluded* — its author is known and the arithmetic can account for them. A
demonstration theme is different in kind: it is the module's own output, so the
entry was not transcribed from a theme anybody found, and what else is in the
directory was assembled the same afternoon by the same hand.

## Context

RFC 0079 put those themes where every fixture in this tree goes, which was right
at the time and is the reason this is a reversal rather than an oversight: the
seven exist to make assertions about `resolve` pass or fail, they are the
argument for the module rather than data about the world, and a test module is
where an argument lives. What changed is that a second reader appeared. `E3-B06m`
built the route `claims/0035` publishes, and that route has to answer a question
the tests never ask: *is this thing I am about to count one of ours?*

The alternatives were live and each is worse in a way worth writing down.

**Refuse by name.** An entry declares `theme = "..."`, so the route could refuse
an entry whose name is one of the seven spellings. It costs nothing and it checks
nothing: the failure mode is somebody assembling a corpus out of what is to hand,
and what is to hand arrives under whatever name the transcriber types. A check
that a dishonest entry passes by renaming itself is a check that only catches the
honest.

**Refuse by declaration.** An entry could carry `source`, and the route could
refuse a source naming this repository. Same defect one level up, and it makes
the check depend on a string a transcriber writes about themselves. `independent`
already asks a question of that shape, and it is asked because there is no way to
compute the answer; here there is.

**Keep them private and duplicate them in `xtask`.** The route could carry its own
copy of the seven and compare against that. This is the version that would have
been written by somebody unwilling to touch `token.rs`, and it is the failure this
repository knows best: two definitions of one thing that agree today. The day an
eighth hostility is argued for, the copy is seven and the module is eight, the
refusal passes the new one silently, and the first evidence is a flattering share.
`docs/postmortem/0001` is about exactly that shape.

**Expose them behind `#[cfg(test)]` on the crate.** Cargo does not build a
dependency's test configuration for its dependents, so this does not exist. Said
here because it is the first thing a reader will propose.

## Consequences

`interface/src/token.rs` grows a public surface whose purpose is to be refused,
which is unusual enough to be worth naming: nothing in this tree is *supposed* to
resolve `SHIPPED[3]` and act on it. It is data about the module, offered so that a
machine can recognise it. The doc comment says so at the constant.

A component pays nothing. Rust materialises a constant where it is used, and the
only user outside the tests is `xtask`, which is not in any image. `cargo xtask
component` reports the compositor at 29 376 bytes before and after this change.

The count is now in one place rather than two, which retires a small stale fact:
`claims/0035` said *five themes — two legal and three hostile* and the module has
carried seven since `HOSTILE_ALIKE` and `HOSTILE_ERASED` were added. The claim
file no longer states a count it would have to maintain; it names the table.

What it makes harder: adding a theme to `token.rs`'s argument is now a change with
a reader outside that file. That is the intended cost. A theme added to the
demonstration is a theme a corpus may not contain, and somebody should have to
know that.

## What would reverse this

**A second table of demonstration themes anywhere.** The whole of this decision's
correctness is that `SHIPPED` and the tests' `THEMES` are one constant. The day
something declares a second list — a fixture crate, a fuzz corpus, a second
claim — the refusal is reading one of them and passing the other, and the answer
is not a third list but a rule about where the first one lives.

**A corpus entry legitimately equal to one of the seven.** `Theme::DEFAULT` is
nine colours and four metrics somebody could plausibly have chosen independently:
white, near-white, near-black, an ordinary blue. If a real theme from a real
system ever collides with one of the seven by value, this check refuses a corpus
it should have admitted, and the argument moves to what else an entry would have
to carry — a source that resolves, most likely — for the collision to be
distinguishable from a transcription. Nothing in this epoch is near that, and the
direction of the error is the safe one: it refuses rather than admits.

**A machine-checkable definition of *who wrote this*.** The `independent` flag and
this refusal are two instruments pointed at one question, and only one of them can
be computed. If the day comes when provenance is a thing a run can verify —
a signature on an entry, a corpus assembled by something outside this tree — then
the value comparison is a special case of a general answer and belongs with it.
