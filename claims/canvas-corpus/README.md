# The corpus behind `claims/0034`

This directory is the corpus `claims/0034-canvas-escape-rate.toml` is a rate
over. It is **empty**, and the emptiness is the finding rather than an omission:

    $ cargo xtask claim canvas-escape-rate
    the corpus is empty, so there is no rate.

`E3-B06l` built the route, the entry format and the refusal. It did not build
the corpus, and could not have: the claim asks for applications **ported by
somebody who did not write the vocabulary**, and everything in this repository
was written by the tree that wrote the vocabulary. A corpus filled in from here
would be the thing `claims/0034`'s own `[workload]` note refuses in advance —
*a vocabulary grading its own homework* — and it would score well.

So this file is addressed to a porter who is not us.

## What an entry is

One `.toml` file per ported application. Files are read in sorted order;
anything that is not a `.toml` file — this README, notes, a screenshot — is
ignored.

```toml
application = "some-editor 4.2, the main window"
source      = "https://example.invalid/some-editor/tree/v4.2"
ported_by   = "a name, or a team"
independent = true
notes       = "what was hard, what was given up on, what a canvas was reached for"

[[node]]
id     = 1
parent = 0
role   = "surface"

[[node]]
id     = 2
parent = 1
role   = "group"
```

`id` is the author-assigned identity, `parent` is the node it sits under, and
`0` is `NodeId::UNNAMED` — a root. `role` is a role's spelling from the
`vocabulary!` invocation in `interface/src/node.rs`: `surface`, `group`, `list`,
`tree`, `item`, `table`, `row`, `cell`, `separator`, `label`, `text`, `image`,
`status`, `command`, `toggle`, `entry`, `choice`, `number`, `canvas`, `track`,
`clip`, `marker`.

Nothing else is recorded — no content, no state, no intent, no tokens. The rate
is a ratio of two counts taken from roles and structure, so a corpus carrying
more would be carrying it for some other claim's benefit and would make porting
more expensive for no gain here. Every tree is put through
`f_interface::node::check` before it is counted, which is what makes the entry a
tree rather than a tally: a canvas under it must declare children, a cell must
sit in a row, and a table's rows must be the same width.

## `independent`, and why the route enforces it

`independent = false` means the porter wrote, or works on, the vocabulary being
measured. Such an entry is read, listed in the run's record, and **excluded from
the rate** — from the numerator and from the denominator both.

This is not politeness. `claims/0034` can only go red in one direction: a rate
under the target is equally consistent with a corpus of applications too simple
to push on anything, and no number in that file can tell those apart. The one
defence it has is who did the porting, so that is enforced where the rate is
computed rather than asked for in prose.

If every entry here is `independent = false`, the admitted corpus has no nodes
in it, `canvas_census` returns `None`, and the run refuses — the same refusal an
empty directory gets, because it is the same state.

*What would reverse this:* somebody arguing that a port by the vocabulary's
author is evidence after all. The argument would have to say what makes it one,
and `claims/0034`'s `[baseline]` notes are where it would have to be made.

## What a good corpus has in it

`docs/design/ring-scene-boot.html` section 13 names the hard cases, and a corpus
without them has measured the easy half of the problem. A timeline. A drawing
surface. A node graph. A spreadsheet. A map. A terminal. A corpus of settings
panels will score beautifully and will have established nothing, and
`claims/0034`'s `[diagnosis]` says so under
`canvas_escape_rate_x1000_far_under`.

Record what was hard in `notes`. The rate is the claim's published number; the
notes are what makes it readable a year later, and the thing RFC 0077 actually
wants to know — *what were the canvases for* — is only ever in them.
