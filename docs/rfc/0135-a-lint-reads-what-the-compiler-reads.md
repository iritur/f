# RFC 0135: A lint reads what the compiler reads, to the end of the file

- Status: accepted
- Date: 2026-09-25
- Affects: `xtask/src/main.rs` — under `lint-datapath`, the new `TestItems`,
  `frame_compile_findings`, `datapath_files`, `datapath_report` and
  `mover_definitions`, with `code_mentions`, `frame_findings` and
  `datapath_findings` changed and `reaches_import` moved onto `lexical_path`;
  under `lint-bounds`, `constant_value` and `constant_terms` over the new
  `const_declarations`; under `lint-exits`, `VERDICTS`, `verdict_stages`,
  `the_sentence_ends`, `carries_exit` and `ends_the_exit` (was
  `opens_paragraph`); under `lint-pulls`, `workflow_pulls` over the new
  `plain_key`. RFC 0127's rule 3 and its *What is normalised* section are
  narrowed by this; `intent/0012-the-interface/spec.md` needs one blank line.
  RFC 0101, RFC 0125, RFC 0126, RFC 0127, RFC 0130.

## Decision

**A textual lint reads the file the way the compiler or the renderer does — code
and not comments, scopes and not columns, every line and not the lines above a
marker — and a word it compares is a word only where a word ends.** Six
findings from one audit, the first in two halves, each run as a mutation
against `84e389c` and each `ok` there:

1. **`lint-datapath` stopped reading a file at its first `#[cfg(test)]`**, in
   four places: `code_mentions`, `frame_findings`, the mover count and
   `datapath_findings`. `#[cfg(test)] const _PROBE: () = ();` above `use
   f_supervisor::policy::fate;` in `kernel/` left every row of `NOT_THE_FRAME`
   green; the same line above a second `stage(` call in a driver left *called
   once* green. **In the frame nothing is excluded now** — `kernel/` is `test =
   false` and has no host tests to skip — and **in a component the item is
   skipped and not the file** (`TestItems`: the item ends at the `}` closing
   its block, or at a `;` or `,` at its own depth). `stamp_findings` had been
   repaired the same way already and was the only one of the five that had.
2. **The same lint could not see a component compiled into the frame from a
   string.** `#[path = "../../user/supervisor/src/policy.rs"] mod judge;` and
   `include!(…)` spell the module only inside a literal, which is stripped
   before a needle looks. `frame_compile_findings` refuses, under a frame
   prefix, a `path` key whose file lies outside that prefix, a `path` key whose
   value it cannot read, and the word `include` in code.
3. **`lint-bounds` matched raw text at column zero.** A decoy `pub const
   MAX_MODULES: usize = 16;` inside `/* */` or a string, above the real one typed
   `core::primitive::usize` and valued nine, resolved to sixteen; and `mod real {`
   left at column zero under `#[rustfmt::skip]` was top level. It now reads
   stripped code, counts braces, and matches `const NAME:` whatever type follows.
4. **`lint-exits` took any bold span as a verdict.** `**advisory only, nothing
   is refused:** <the sentence>` passed on the real tracker. A verdict is now
   one of `**met.**` and `**not met.**`, and there is at most one.
5. **A wrapped exit ended at a bold lead or a list marker.** Now it ends only at
   a `*field:*`, a blank line or a fence.
6. **The word boundary after a sentence was not one.** `seed-less`,
   `seed_never` and `seed's` passed as `seed`. The sentence now ends only at the
   end, at whitespace, or at one mark of punctuation followed by either.
7. **`lint-pulls` read `container:` and `services:` as written.** `container :
   img`, `"container": img` and `'services':` were seen by neither the job reader
   nor the count by text. Every key is put in its plain spelling before either
   reads it.

## Context

RFC 0130 was the last round of the same audit and its *What none can see* named
the `#[path]` module; the auditor this time ran the mutations rather than
reading for them. Every one of the seven is the RFC 0101 shape one level up: a
check that decays silently, in the direction that reports ok.

The alternatives were live for three of them.

- **For the frame's `#[cfg(test)]`, refusing the attribute or skipping the
  item.** Neither: the frame reads everything. A refusal adds nothing the
  needles do not already see once the scan reads through, and a skip would be
  an exclusion for tests the frame cannot have. The component side has tests in
  every file, so it skips the item.
- **For the verdict, a pattern — *starts with `met` or `not met`* — or a
  list.** A list: `**met on x86-64.**` starts with `met` and is a narrowing.
  The tracker was read for every bold lead in use: eighty-two lines, each with
  one span; every compared line uses `**met.**`; nineteen other forms on
  twenty-three lines close lines in epochs with no decomposition spec and are
  not admitted.
- **For the wrap, keeping `- ` as a paragraph break**, which is what Markdown
  renders. Rejected on the direction of the failure: a list inside a sentence
  that is taken as the sentence's end is a silent pass, and one taken as part
  of it is a red finding a blank line repairs.

## Consequences

The seven probes are red, and each check has a mutation attacking it that turns
its own tests red; `datapath_report` exists so that deleting a call site is one
of them, which is the defect RFC 0084 recorded in the licence boundary.

**The wrap rule costs one blank line today.** A paragraph a peer added under
`E3-B05e`'s exit in `intent/0012-the-interface/spec.md` on 2026-09-25 begins
`**Narrowed in its meaning, not its words, by RFC 0129 …**` directly under the
exit line. Under the old rule the bold lead ended the exit; under this one it
is lazy continuation — which is how Markdown renders it too — and the spec's
sentence grows by the paragraph, so the tracker no longer carries it and
`lint-exits` is red. The repair is a blank line above that paragraph. Its words
do not change, and nor does what it says.

**Verdicts that narrow are refused on compared lines**, and on no other line
yet. The day E1 or E2 is decomposed, `**met, narrowed under RFC 0093.**` on one
of its lines becomes a finding. That is the argument RFC 0084 already makes:
the narrowing belongs in the spec.

**What stays open**, and each is a change outside the file that a reviewer
reads: a macro exported by a crate the frame links that expands to a call into
a component; a `#[path]` or `include!` inside a crate the frame links rather
than in `kernel/`; a build script generating source (RFC 0092 refuses those on
its own terms); a test item written as `fn f<A, B>()`, whose comma ends the skip
early — the safe direction for a forbidden call and not for a definition count;
a flow-style workflow, a `? key` and a YAML merge key; and a narrowing written
after the sentence as evidence, which RFC 0127 already names.

## What would reverse this

- **The frame gaining host tests.** Then `frame_findings` skips the item
  through `TestItems`, as the component side does — never the file.
- **A third verdict a reviewer needs on a compared line** — `**dropped.**` on a
  `[~]` child — joins `VERDICTS` in the diff that first needs it. A phrase that
  says which part holds does not.
- **An exit that must be followed, with no blank line, by a list that is not
  part of it.** That is a shape to restructure, and if it recurs, a `*field:*`
  for it.
- **A textual scan replaced by one over `rustc`'s own expansion** — a
  `-Zunpretty=expanded` pass, or a `syn` parse. Then `TestItems`,
  `const_declarations` and `frame_compile_findings` go, and the stated limits
  with them.
