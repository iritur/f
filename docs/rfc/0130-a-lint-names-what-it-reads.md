# RFC 0130: A lint names what it reads, not a word that happens to be there

- Status: accepted
- Date: 2026-09-25
- Affects: `xtask/src/main.rs` — `NOT_THE_FRAME` gains a fourth field and a
  tenth row, `lint_datapath` reads the fourth field; `constant_value`,
  `constant_terms` and the new `bound_spellings` under `lint-bounds`, and the
  `MAX_RESERVED` row's `spelled_as`; `workflow_jobs`, `workflow_job_body`,
  `workflow_pulls` and `lint_pulls`, with `jobs_block`, `job_key`,
  `indent_of`, `yaml_quiet` and `PULL_KEYS`. RFC 0101, RFC 0116, RFC 0122,
  RFC 0123, RFC 0125, RFC 0126.

## Decision

**A check that looks for the absence of something must hold the presence of
exactly that thing, spelled so that nothing else can answer for it.** Three
lints were each answering a narrower question than the one they reported on,
and each is changed to name what it reads:

- **`lint-datapath`'s module rows.** A `NOT_THE_FRAME` row is now `(the prefix
  that must not name it, the needle, the prefix that must, what it must say
  there)`. For the two rows whose needle is a module's name, `policy` and
  `inbound`, the fourth field is the module's declaration — `pub mod policy;`,
  `pub mod inbound;` — rather than the name, which `pub policy: Policy` in
  `user/supervisor/src/routing.rs` and `let mut inbound =` in
  `user/compositor/src/component.rs` answered for over a renamed module. A
  tenth row refuses `TIMEDOUT` in `kernel/` code, because a frame that judged a
  timeout inline and packed the word would name no module at all.
- **`lint-bounds`' resolver.** A term in a bound's sum resolves only when its
  spelling names the declaration the check read: bare for one in the same
  file, the declaring file's module path (with or without `crate::`) for one
  elsewhere in the crate. The declaration read must sit at the top level of its
  file. `MAX_RESERVED` must be spelled through
  `arch::x86_64::multiboot::MAX_MODULES` in full, not through its last segment.
  This reverses RFC 0116's resolver, which read other constants *taken by the
  last path segment*.
- **`lint-pulls`' reader.** Job keys are found at the column the `jobs:`
  block's first key sits at, whatever it is, the way `has_schedule` finds
  `schedule:`; a job's body is re-indented so its own keys are at four spaces
  for the helpers that read them there; `services:` counts as a pull beside
  `container:`; and each file's `container:` and `services:` keys are also
  counted by text, so that a key no job the reader found holds is a finding in
  that file.

## Context

An adversarial audit read the three lints without running them and built a
mutation against each; every one of them was run on 2026-09-25 against
`09b4262` and reported ok:

- `pub mod policy` renamed `pub mod judge`, and a new file in `kernel/`
  calling `f_supervisor::judge::fate`: `lint-datapath: ok`. So was a `kernel/`
  file writing `abandoned > seen && waits > 0` and packing
  `f_abi::control::cause::TIMEDOUT` — which is the decision RFC 0123 puts above
  the frame, taken in the frame. The same rename done to `pub mod inbound` was
  also `ok`, found while repairing the first; it is the same defect in the same
  table.
- `mod pinned { pub const MAX_MODULES: usize = 13; }` beside
  `const MAX_RESERVED: usize = FIXED_RESERVED + pinned::MAX_MODULES;` in
  `kernel/src/main.rs`, and separately
  `use crate::component::PLACES_MAX as MAX_MODULES;` with the sum spelled
  bare: `lint-bounds: ok` both times, computing five plus sixteen while the
  compiler built five plus thirteen and five plus nine. That is RFC 0101's
  decay inside the check written against it, which `constant_value`'s own
  documentation had already warned of once.
- A workflow indented at four spaces holding a container job whose
  `permissions:` drops `packages: read`, and a two-space workflow holding the
  same job with `services:` instead of `container:`: `lint-pulls: ok`, because
  the first had no jobs to its reader and the second had no container.

The auditor's proposed repair for `lint-bounds` was a workspace scan refusing
any `MAX_MODULES` declaration or `as MAX_MODULES` alias outside `multiboot.rs`.
It is not what landed, because the resolver repair holds more with less: a
declaration or alias elsewhere only matters if some spelling in a bound reaches
it, and every spelling the resolver now accepts reaches the one top-level
declaration it read — while a scan would have had to be taught that
`user/supervisor/src/routing.rs` legitimately declares a `PLACES_MAX` of its own.
The resolver also closes a case the scan would not have: a bound summed through
`f_supervisor::routing::PLACES_MAX`, which the last-segment reader would
resolve to the frame's nine in any bound read after `PLACES_MAX`. Today that is
refused only because `BOUND_SOURCES` happens to read `MAX_MODULES` first — an
order, not a rule.

## Consequences

Easy: renaming either module goes red in `lint-datapath` until the row is
renamed with it, which is the day somebody reads the row. A bound's spelling is
now the thing the check compares, all the way to the file. A workflow can be
indented however a runner accepts and still be read, and a shape the reader
cannot read is a red line naming the file rather than a pass.

Hard: `kernel/` may not spell `TIMEDOUT` anywhere in shipped code — the frame
checks the word by `cause::named_above(cause::of(..))`, which names nothing,
and a log line wanting the word must go through `cause::label`. A module
declared inline (`pub mod policy {`) is refused as absent. A boot-path bound
spelled through another crate is refused until `bound_spellings` is taught that
crate's name. A `container:` or `services:` line in a `run:` script is counted
by text and would be red.

Forecloses: nothing a boot does. The three lints read source; none of them is
on a boot path.

What none of the three can see, stated so nobody believes otherwise:
the literal `5` in place of `TIMEDOUT`, or an inline judgement written onto the
wire as another cause; a `#[path]` attribute moving a module off the file its
path names; a workflow written entirely as a flow mapping, whose keys are not on
lines of their own for either count to find.

## What would reverse this

For the fourth field: a module this tree needs to declare inline or re-export
under another name, at which point the field becomes whatever that declaration
is — never the bare name again. For the `TIMEDOUT` row: a second cause that is
a judgement (`cause::named_above` gains an arm), which adds a row, or a frame
that legitimately has to spell the word — which would be RFC 0123 reversed, and
belongs in an RFC that says so before it belongs here. For the resolver: a
bound that must be summed through another crate, which teaches
`bound_spellings` that crate's name rather than relaxing it to the last
segment. For the job reader: a workflow in flow style, which is a reader this
file does not have and should get before such a workflow lands.
