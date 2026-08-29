---
id: 0000
status: draft        # draft | in-progress | done | abandoned
spec: ./spec.md
---

# Plan: title

Written with the agent, from an agreed `spec.md`, before any code exists. Short
on purpose: the point is to agree the shape of the diff while it is still free
to change.

## Files

Every path this change touches, one line each. New files marked. A path missing
from this list is a review finding; a path listed but never touched is a sign
the plan was written about a different design than the one built.

```
path/to/file.rs        what changes here, and why this file rather than another
path/to/new_file.rs    NEW: what it is for
```

## Order

The sequence, and why it is that sequence. Usually: whatever can fail first. A
plan that starts with the easy part and leaves the risky part for step five is a
plan that finds out late.

1.
2.
3.

## Proof

The command that says this worked, and the state of the tree before it says so.
For a bug fix the failing test comes first and the diff that makes it pass comes
second, so that the test is written before the cause is understood and therefore
tests the bug rather than the fix.

```
cargo xtask verify
```

## Risks

What could go wrong during the build, as distinct from what could be wrong about
the design — that belongs in `spec.md`. Ordering bugs, a baseline that has to be
re-measured, a document that has to be rewritten in the same commit.
