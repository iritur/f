# ops/

The maintain stage: what is watched, what a deviation is allowed to cause, and
where the output goes.

```
bands.yaml     the signals, and what each band permits
detect.sh      deterministic detection. No model involved in deciding something moved.
metrics/       the histories, one CSV per signal
```

## The shape of the loop

    detect.sh  ->  a band  ->  what that band permits  ->  intent/  ->  a person

Detection is a standard deviation, not a judgment. That matters more than it
looks: an agent asked to watch a dashboard will find something to say every time
it looks, and a stream of plausible findings is indistinguishable from noise
after the second week. So the model is not consulted about whether something
moved — only about what it means, and only once arithmetic has said it did.

The bands are in `bands.yaml`. One standard deviation logs. Two permits a
read-only diagnosis that ends in an `intent.md`. Three permits a pull request or
a pre-approved runbook. Nothing escalates past that: there is no band in which
an agent deploys, force-pushes or edits a claim threshold, and
`.claude/hooks/release-gate.sh` does not read this file and cannot be overridden
from it.

## The histories

`metrics/<signal>.csv`, two columns: an ISO-8601 UTC timestamp and the value.
Comment lines start with `#`. Append-only — a history that gets rewritten is a
history that cannot detect anything.

At M0 most of these do not exist, because the numbers do not exist. `detect.sh`
says "no history yet" and moves on, which is the honest output and deliberately
not an error. The apparatus is wired before there is anything to watch for the
same reason `claims/0001` was registered before it was measurable: retrofitting
it later means retrofitting it onto a system that has already learned to live
without it.

Fewer than eight samples reports "too few to say anything". A z-score over three
points is a number with the shape of evidence and none of the content.

## Running it

```bash
bash ops/detect.sh
```

`.github/workflows/maintain.yml` runs it daily and acts on the `max-band:` line.
Running it by hand is the same thing and is how you check what the workflow saw.

## Adding a signal

Add an entry to `bands.yaml` and start appending to its CSV. Two questions
first, because a signal nobody acts on trains everybody to ignore the report:

1. **What would you do differently if this moved two standard deviations?** If
   the answer is nothing, it is a metric, not a signal, and it belongs on a
   dashboard.
2. **Is it deterministic?** A signal that varies with runner load produces bands
   that mean runner load. `ci-duration-seconds` is watched anyway, for a stated
   reason, and it is the exception rather than the pattern.

## When something fires

Band 2 and 3 both end in an `intent.md` written by
`.claude/agents/incident-intent.md`, in the format `intent/README.md` defines,
with the evidence quoted rather than paraphrased. It arrives as a draft. A
person triages it: fix now, schedule, or dismiss with a reason.

And then the part that is easy to skip — the incident becomes a task in
`evals/`, in the same change that fixes it. An incident that produced no eval
will happen again in a form nobody recognises.
