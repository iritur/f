---
id: 0000
status: draft        # draft | agreed | superseded
reviewed_by:         # the originator, by name, before plan.md starts
skills:              # which .claude/skills applied while this was written
---

# Spec: title

One paragraph restating the intent in the repository's own terms. If this
paragraph runs longer than the intent it summarises, the intent was vague, and
upstream is where to fix that.

## Behaviour

What the system does afterwards, precisely enough to test. Name the observable:
an exit code, a returned error variant, a rendered number, a boot that reaches a
stage it did not reach before.

## Policy applied

Which standing policies bear on this change, and what they force. Not a recital
— only the ones that actually constrain the design. If determinism forbids the
obvious implementation, say what the obvious implementation was, so that the
next person does not rediscover it.

## Not in scope

What this deliberately does not do, and where that work goes instead: another
intent, a later milestone, an RFC. This section is what stops a spec from
growing during the build.

## Evidence

How anyone knows it worked. One of:

- a test, named;
- a claim in `claims/`, with a threshold;
- a document that can say something it could not say before.

"Manual check" is not evidence, and does not survive the person who did it.

## Risks and reversal

What is most likely to be wrong here, and what observation would say so. If this
change encodes a decision somebody could reasonably re-litigate, it needs an RFC
as well — say which.
