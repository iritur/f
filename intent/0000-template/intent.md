---
id: 0000
status: draft        # draft | accepted | withdrawn | shipped
originator:          # who wants this. A name, not a role.
todo:                # TODO.md task IDs, once there are any
---

# Title, as the person asking would say it

## Problem

What is wrong or missing now, written so that somebody who disagrees can say so
without first having to guess what you meant. No proposed solution here — if the
problem cannot be stated without naming the fix, then the fix is what is really
being argued for, and this section is decoration.

## Proposed outcome

What is true once this exists. Observable where possible: a number, a behaviour,
something somebody can do that they could not do before.

## Affected users and systems

Which crates, which documents, which people. Name the `docs/design/` pages that
would have to change — those are expensive, and finding out late is the usual
way an estimate turns out wrong.

## Constraints

What is not allowed to move. The three policies in `CONTRIBUTING.md` constrain
everything and do not need restating; this section is for the constraints
particular to this change — a milestone, a wire format that is frozen, a
baseline that has to stay comparable.

## Open questions

What is genuinely undecided. An empty section here usually means the questions
were answered by assumption rather than by asking, so write them down even when
they feel small.
