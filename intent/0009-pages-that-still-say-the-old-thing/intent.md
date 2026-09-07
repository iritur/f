---
id: 0009
status: draft        # draft | accepted | withdrawn | shipped
originator: Dmitri Chudinov
todo:                # TODO.md task IDs, once there are any
---

# The design pages still say things we have since decided are wrong

*Filed from the adversarial audit of the E2 branch, 2026-09-07, with intents
0007 and 0008.*

## Problem

The design pages are the reasoning, and they are allowed to be ahead of the code
— that is the deal, it is written down, and it is what makes them worth writing.
They are not allowed to be *wrong*, and eight sentences across them currently
are: each states a decision that this project has since reversed, in an entry
that was accepted, that names the sentence it reverses, and after which nobody
edited the page.

Three examples, so that the shape is clear rather than asserted. A storage page
says atomicity is free because publishing a root is a single write; we have
since decided that it is a sequence of writes with barriers between them, that
the barriers are the cost, and the entry that decided it quotes that sentence
by name. The same page's honest paragraph about the workload the design is bad
at calls the mitigation a patch rather than a resolution; we have since decided
it is a second object kind and not a patch. A metrics table still gives full
system rollback as one reboot; we have since decided the metric is one
generation swap, with a reboot only when the frame changed — that change of
metric is one of the reasons the epoch exists.

None of these is a lie anybody told. Each is a sentence that was true when it
was written, was reversed on purpose in the right place, and was left behind
because reversing a decision and editing the page that argues for it are two
different acts and only the first one is required of anybody. The reversal
entries even name the page and the section, which means the information needed
to fix each one has been sitting in the tree since the day the decision changed.

The cost is specific and it is the reason this is worth an intent rather than a
chore. A reader who starts at a design page — which is what we hand people, and
what we have made pleasant to read for exactly that reason — and who never opens
the reversal directory reads a decision we no longer hold, argued well, with no
mark on it. That is the *most* convincing possible way to be wrong. It also
quietly inverts the deal: the pages are supposed to be ahead of the code and
these sentences are behind it, so the one place a reader is told to expect
optimism is instead where they find the past.

It will get worse rather than better on its own. Reversals are cheap to write
and are meant to be; every future one is another chance to leave a page saying
the old thing, and there is nothing in any check we run that would notice.

## Proposed outcome

- No design page states a decision that an accepted reversal has overturned.
  Where the page keeps the old sentence deliberately — because the argument is
  worth reading and the outcome changed — it says so and points at what
  reversed it, rather than reading as current.
- A reversal that leaves its page stale is caught by the same loop that already
  catches a reversal condition that has fallen due and not been paid. We already
  have the idea that an obligation can go unpaid and be reported; this is the
  same idea applied to prose that a decision falsified.
- The numbers on those pages keep coming from the registry rather than being
  edited in place, which is already the rule and must survive this.

Observable when it is done: deliberately reversing a decision without touching
the page it contradicts fails a command, and the failure names the page, the
section and the entry that reversed it.

## Affected users and systems

- All five `docs/design/` pages, and most heavily the storage one, which is
  where this epoch's reversals landed.
- `docs/rfc/`, whose entries already name the document and section they affect —
  that field is what makes this checkable at all, and if it is to be load
  bearing it has to be written to a shape rather than to taste.
- `xtask/`, if the check becomes a command, and `CONTRIBUTING.md`, which is
  where we say which rules are mechanised and which are review.
- Anybody who reads the design pages, which is the audience the pages exist for
  and the one that cannot see this problem.

## Constraints

- **The pages stay ahead of the code on purpose.** Whatever is built must not
  turn into a rule that a page may only describe what exists — that would delete
  the thing the pages are for and is the obvious wrong repair.
- A page is not edited to change a published number. Numbers there come from the
  registry and that stays true.
- Reversal entries are not rewritten to fit a checker. If the field a check
  would read is not currently written to a usable shape, the shape is the thing
  to argue about, and changing decades of prose to please a lint is the failure
  mode to avoid.
- Eight is what one audit found on one branch. Nobody should assume it is the
  whole list, and the change should not be sized as though it were.

## Open questions

- How much of this can actually be checked by a machine? Naming a page and a
  section is checkable; knowing whether a sentence still says the reversed thing
  is not, and the honest mechanism might be *this page has an unreviewed
  reversal against it* rather than *this sentence is wrong*.
- What does a page do with an argument that was good and whose conclusion
  changed? Deleting it loses the reasoning, which is the one thing these pages
  exist to keep. Marking it costs the reader something on every future read.
- Do the reversal entries' *affects* lines need a shape before any of this is
  possible, and is that a separate change that should land first?
- Is there a similar drift between the pages and the claims registry — a
  published number whose claim has since moved — and should it be looked at in
  the same pass rather than found by the next audit?
