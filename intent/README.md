# intent/

Where a change starts, before it is a diff and before anyone has agreed it is a
good idea. One directory per change, three files, in the order they are written:

```
intent/NNNN-short-name/
  intent.md    what somebody wants, in their own words
  spec.md      what we agreed it means, with the policies already applied
  plan.md      how it gets built, named file by file
```

The directory is the record. Git history carries the author, the timestamp and
the approval, so none of those are fields in the files.

## Why the files are separate

They are written by different people at different times and they fail
differently. An `intent.md` that is wrong wastes a conversation. A `spec.md`
that is wrong wastes a week. A `plan.md` that is wrong wastes an afternoon.
Collapsing them hides which one was wrong.

The other reason is that `intent.md` is the only one of the three that a
non-engineer writes, and it should stay writable by someone who has never run
`cargo`. Do not ask it for file paths.

## The rules

1. **`intent.md` is not edited to match what was built.** If the built thing
   differs, that belongs in `spec.md` or in an RFC. The intent records what
   somebody wanted at a moment, and rewriting it destroys the only evidence of
   whether we build what people ask for.
2. **`spec.md` goes back to the originator before `plan.md` starts.** The review
   that matters is the one that happens before code exists, because it is the
   only one that is still cheap.
3. **`plan.md` names files.** A plan that does not name the files it will touch
   has not been thought about; it has been agreed to.
4. **The diff matches the plan.** Extra files are a review finding. So is a plan
   step with no diff behind it. `REVIEW.md` pass 4.
5. **Numbering is permanent.** `0003` means the same thing forever. An abandoned
   intent stays in place with `status: withdrawn` and one line saying what was
   learned — the same rule `TODO.md` uses for `[~]`, for the same reason.

## Linkage

This repository is authoritative. `TODO.md` task IDs are the only other
identifier a change carries, so:

- an intent that becomes work names the task IDs in `todo:`,
- and the `TODO.md` task names the intent on its line.

Either one alone rots. Both together mean you can get from a ranked task to the
argument for it in one step, which is the whole reason the linkage exists.

Intents that produce a decision get an RFC (`docs/rfc/`). Intents that produce a
number get a claim (`claims/`). An intent that produces neither produced a
patch, which is fine and needs no ceremony.

## Where intents come from

Three sources, and the third is the one this directory exists for:

- a person, in conversation;
- a review finding too large to fix inside the pull request (`REVIEW.md`);
- an agent: a monitoring band in `ops/bands.yaml`, the weekly security scan, or
  a production incident. Those write `intent.md` in exactly the format below and
  nothing else. An agent does not get to write its own spec.

Triage is a human step in all three cases. `docs/sdlc.md` says whose.
