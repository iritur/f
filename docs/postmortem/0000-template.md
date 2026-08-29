# Post-mortem NNNN: title

- Date of the incident:
- Date of this document:
- Detected by: a person, a band in `ops/bands.yaml`, a CI job, a scan — say which
- Commits: caused by `<sha>`, fixed by `<sha>`

## What happened

Plainly, in order, with timestamps where they exist. Written so that somebody
who was not there can follow it without asking a question.

## What made it possible

The technical cause. Not the person, not the hurry, not the review that missed
it — those come later and are usually consequences of this.

## What made it invisible

The second failure, and usually the more expensive one. How long it existed
before anybody noticed, and what was supposed to have noticed. A check that did
not exist, a check that existed and did not fire, a signal nobody watched, a
report everybody had learned to scroll past.

## What was true that we believed was not

The assumption. Every incident has one, and it is what makes the document worth
reading in a year — the mechanics will have changed and the assumption will
still be being made somewhere.

## What changed

Each line names something that runs, and links it:

- lint / hook / test / eval / band:
- the RFC, if a decision changed:
- the claim, if a number was wrong:

If nothing here can be filled in, say why. "We were careful" is not a change,
and the next occurrence will not be recognised as the same thing.

## What we chose not to change

The fixes considered and rejected, with the reason. This is what stops the next
post-mortem re-proposing them, and what makes it obvious if the reason has
stopped being true.
