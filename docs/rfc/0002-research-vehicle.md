# RFC 0002: The target is a research vehicle, not a product

- Status: accepted
- Date: 2026-08-27
- Supersedes: the workstation product target
- Affects: architecture document sections 12-13, every claim in `claims/`

## Decision

F is a research vehicle whose deliverable is defensible measured claims. It is
not a product and has no obligation to acquire users.

## Context

An earlier target was a professional audio and video workstation with concurrent
local inference. That target was coherent and had a real market wedge. It was
abandoned because the constraint it carried — a shipping product needs an
ecosystem — is exactly the constraint that has defeated every clean-slate
operating system, Fuchsia most recently and most expensively.

Research systems are judged differently. Exokernel, Barrelfish, Arrakis,
Singularity and Theseus between them had approximately no users and changed how
a generation of systems were built.

## Consequences

The ecosystem wall stops being the governing risk, which frees the architecture
to take positions a product could not. In exchange the bar moves onto
measurement: a claim without a named baseline, a published workload and a
reproduction command is not a result.

This is why `claims/` exists and why it gates CI. It is also why the baseline
must be a *tuned* Linux — beating a stock configuration is the most common way
systems research becomes worthless, and it is entirely self-inflicted.

## What would reverse this

A workload where the architecture's advantage is large enough that users would
tolerate the ecosystem gap. That would be a product decision, and it would
reintroduce every constraint listed above.
