---
id: 0008
status: draft        # draft | accepted | withdrawn | shipped
originator: Dmitri Chudinov
todo:                # TODO.md task IDs, once there are any
---

# The four numbers this epoch promised would gate, and none of them does

*Filed from the adversarial audit of the E2 branch, 2026-09-07, alongside
intent 0007. That one is about bounds nothing compares; this one is about
numbers that were going to be the point of the epoch and are all still owed.*

## Problem

When this epoch was specified we named the numbers it would produce and said,
for four of them, that they would gate the build: write amplification against a
tuned Linux baseline on the same device, bytes re-chunked per byte written,
copies per read, and resident bytes per unit of work. Those four are close to
the whole of what the epoch claims about storage — they are the answers to *does
content addressing cost more than a filesystem*, *is an edit cheap*, and *does a
read copy*. Every one of them is recorded today as not measured and not gating.

Each has a reason and none of the reasons is dishonest. Two are waiting on a
component that has not been built, so the boundary the number is defined at does
not exist yet. One is waiting on an emulator newer than our development image
carries, and on a baseline configuration that has been written down and never
run. One is measured, but at a boundary one layer below the one its own
definition names, and the file says so rather than promoting itself. The
registry is behaving exactly as designed here: it refuses to call a number
evidence when it was taken somewhere else.

The problem is not the honesty. It is that four *pending* rows with four good
reasons and no owner add up to something nobody has said out loud: the epoch's
storage story is currently argued rather than measured, the release that is
supposed to package these claims has no line saying which of them will be empty
on the day it ships, and there is no point at which anybody is required to look
at the four together. Individually each file reads as a debt somebody is
tracking. Together they read as a plan that quietly stopped being a plan, and
the difference is only visible if you open all four.

There is a second-order cost. Two of the four have thresholds published with no
measurement behind them — deliberately, so that a first run cannot fit the
threshold to itself — and a threshold that has stood unmeasured for a whole
epoch starts to look like a result to anybody skimming.

## Proposed outcome

- The four are looked at as one set, once, by a person, and each comes out with
  one of three answers: it gates now; it gates on a named day, with a task whose
  exit says what must exist first; or it is withdrawn as a promise of this epoch
  and the design page that sells it is corrected in the same change.
- Whatever the release ships with, it names the numbers it does not have. We
  already require a release to state what it does not contain; this is that rule
  applied to the numbers rather than to the features.
- Anybody reading the registry can tell, in one command, which claims were
  promised for the current epoch and which of those have numbers — without
  opening four files and reading four paragraphs.

Observable when it is done: for each of the four, either a measurement in the
file and a status that gates, or a task ID on the claim's line and an exit
condition somebody could close.

## Affected users and systems

- `claims/`: the four entries, and probably the registry's own README, since
  *pending* is currently doing two jobs — *waiting for a machine* and *waiting
  for a component that does not exist* are not the same wait.
- `intent/0006-state/spec.md`, which is where the four were promised, and which
  is the document that has to record the answer rather than being quietly left
  behind.
- `TODO.md`, for whichever of the four turns into a task with an exit.
- The release task for this epoch, which is where the *what is not measured*
  list has to appear.
- `docs/design/deadline-all-the-way-down.html` if any of the four is withdrawn:
  that page is where the storage argument is made and it should not keep selling
  a number the tree has stopped intending to take.

## Constraints

- **No number is invented to fill a row.** A ratio against a baseline nobody
  ran, or a count taken at a boundary the claim does not name, is worse than an
  empty row: an empty row is a debt and a wrong row is evidence.
- No threshold is fitted to a first measurement. The thresholds were written
  before the runs on purpose and that decision stands.
- The environment refuses to record timings and that is it working, not an
  obstacle to route around. This intent is about the counts.
- Whatever is decided must not turn into a fifth honest paragraph. The failure
  mode here is another page of true sentences and no verdict.

## Open questions

- Is *pending* one state or two? *Waiting for a machine this project does not
  own* and *waiting for a component this project has not written* fail
  differently and are answered by different people.
- Does the epoch's release wait for any of the four, or does it ship and say
  what is missing? The intent behind this epoch already asked a version of this
  question about one of its build tasks and did not answer it.
- If a number cannot be taken this epoch, does the design page that sells it
  lose the sentence or gain a date? We have a strong rule about not overclaiming
  and no rule at all about how a promise is allowed to age.
