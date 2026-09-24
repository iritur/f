# The corpus behind `claims/0035`

This directory is the corpus `claims/0035-theme-refusals.toml` takes its two
numbers over. It is **empty**, and the emptiness is the finding rather than an
omission:

    $ cargo xtask claim theme-refusals
    the corpus holds no theme this run may count, so there is no share.

`E3-B06m` built the route, the entry format and the refusals. It did not build
the corpus, and could not have: the claim asks for themes **written by somebody
who is not working on `interface/src/token.rs`**, and everything in this
repository was written by the tree that wrote that module. A corpus filled in
from here is the thing `claims/0035`'s own `[workload]` note refuses in
advance — *a share computed over the answers* — and it would score beautifully.

`claims/canvas-corpus/README.md` is this file's sibling and says the same thing
about a different number. The two were built one wave apart by two people who
each reached the same conclusion, which is worth more than either reaching it
alone.

So this file is addressed to a theme's author, who is not us.

## What an entry is

One `.toml` file per theme. Files are read in sorted order; anything that is not
a `.toml` file — this README, notes, a screenshot of the theme in its own system
— is ignored.

```toml
theme       = "some-desktop 4.2, the dark theme it ships"
source      = "https://example.invalid/some-desktop/tree/v4.2/theme.json"
written_by  = "a name, or a team"
independent = true
notes       = "what it was written against, and what it looked like here"

[colour]
surface_one  = "#121212"
surface_two  = "#1E1E1E"
field        = "#0A0A0A"
text         = "#E8E8E8"
text_muted   = "#A0A0A0"
emphasis     = "#7FB3FF"
edge         = "#6E6E6E"
field_text   = "#E8E8E8"
field_danger = "#FF8A80"

[metric]
text_size_pt_tenths   = "110"
density_per_thousand  = "1250"
space_em_per_hundred  = "62"
stroke_em_per_hundred = "8"

[font]
first  = "Source Sans 3"
second = ""
third  = ""
```

Every key above is required and none has a default. A default is what admits a
theme half-transcribed, and a share over half-transcribed themes is a share over
this format's defaults rather than over anybody's theme.

The nine colours are the nine tokens the resolver knows — three grounds and six
inks — spelled as `#RRGGBB`. The four metrics are the four a theme may set, each
with its scale in its name: tenths of a point, thousandths, hundredths of the em,
hundredths of the em. The three font slots are preferences, most preferred first;
an **empty** slot is absence rather than a fault and is skipped without a note,
because a theme naming one family has to spell the other two somehow.

### Why the metrics are quoted

Because this repository's TOML subset reads unsigned integers only, and a corpus
that could not spell `space_em_per_hundred = "-30"` would be a corpus with every
theme that asks for something impossible quietly left out of it. That is not a
error in the share — it is the share moving towards its ceiling, which
`claims/0035` says would mean *the apparatus is ceremony*. A quoted decimal with
an optional sign can spell every value the type holds, so the format does not get
to choose which themes are measurable.

*What would reverse this:* `xtask/src/manifest.rs` reading a signed integer, at
which point these become integers like every other number in the tree.

### Why the scales are spelled in words

`text_size_pt_tenths` and not `text_size_pt_x10`, which is what the field it
fills is called. A bare key in this repository's TOML subset is lower-case letters
and underscores with no digits in it, so `_x10` is not a key this reader can hold,
and the choice was between widening a grammar four readers share for one corpus's
convenience — `xtask/src/manifest.rs`, `docs/manifest.md`, and every manifest in
the tree — and spelling the scale as a word here. RFC 0004 asks for the scale in
the name and not for a particular suffix; `text_size_pt_tenths` says exactly what
`text_size_pt_x10` says.

*What would reverse this:* a second corpus that wants a digit in a key, at which
point widening the grammar is a change two callers ask for rather than one bent
for the convenience of one.

## What the run refuses, and why each refusal is arithmetic

**An entry that does not parse.** Not counted as zero and not skipped: either
would move the share — one by shrinking the denominator, the other by leaving a
theme's author out of the number in silence — and the share is the one thing in
this claim nobody can check by reading it.

**An entry that is one of this module's own demonstration themes.** `SHIPPED` in
`interface/src/token.rs` is the seven themes that module carries: two legal and
five hostile. They are an argument about the resolver, not a sample of the world,
and a share over them is a share over the answers. The run compares every entry
against all seven **by value** and refuses the corpus when one matches — by
value, because a name is something an entry writes and a value is not. RFC 0110
is why
those seven are published rather than hidden in the module's tests: a claim that
cannot recognise them cannot refuse them, and a guard that exists only as a
sentence in a claim file is a guard this repository has twice recorded the fate
of.

One matching entry refuses the whole corpus rather than being dropped from it. A
corpus with a demonstration theme in it was not assembled from themes somebody
found; it was assembled from what was to hand, and the rest of it is owed the
same suspicion.

**`independent = false`.** The author wrote, or works on, the resolver. Such an
entry is read, listed in the run's record, and **excluded from both numbers**.
This is arithmetic rather than manners: a share of clean themes is evidence about
this layer's floors only if the themes were written against something other than
those floors, and enforcing that where the number is computed is the difference
between a rule and a request.

**A corpus with nothing left in it.** If the directory is empty, or every entry
declares `independent = false`, `f_interface::token`'s `census` answers `None` and
the run refuses. It is the same refusal from the same function in both cases,
because it is the same state. A clean share here would report a number about a
corpus nobody assembled, and would do it in the reassuring direction.

## What the run records beside the numbers

Every entry: its file, the theme, its source, who wrote it, whether it is
independent, what it notes, and what resolving it did — clean or not, how many
decisions, how many of those did not fit in the report.

Then the **content hash** of the corpus: the SHA-256 of every entry the directory
held, each one's path and then its bytes, in sorted file order. `claims/0035`'s
`[workload]` asks for it and says why — *no theme was added or removed after a
run* is a condition that decays quietly, so the run records what it read rather
than asking a reader to trust that the directory has not moved since.

Independent or not, because an entry excluded from the numbers still has to be
accounted for. Path and then bytes, because a file swapped for another with the
same palette is a different corpus and a hash over the values alone would not
notice. Over an empty directory it is the SHA-256 of nothing, which is a
recognisable constant and the honest answer to *what did you read*.

Then the distribution of note kinds — contrast raised, contrast unreachable,
metric clamped, font dropped. `claims/0035`'s `[diagnosis]` says a red share
cannot be debugged without it, so it is printed on every run rather than added on
the afternoon the share goes red.

## What a good corpus has in it

Themes somebody shipped, from systems that are not this one, written against
screens their authors looked at. A dark theme and a light one from the same
system, because a pair is where a palette's author had to make the same decisions
twice. A high-contrast theme, which is the case where this layer should have
nothing to do. A theme from a system with no contrast checking in it at all,
which is most of them — RFC 0079's `Context` is built on that observation, and
this claim is the first thing in this tree that could put a number on it.

A corpus of themes written *against* these floors will score near a thousand clean
and will have established nothing, and `claims/0035`'s `[diagnosis]` says so
under `themes_resolving_clean_per_thousand_over`. That is the more likely failure
here, and it is the one the `independent` flag exists to make visible.

Record what the theme was written against in `notes`. The share is the claim's
published number; the notes are what makes it readable a year later, and the
thing RFC 0079 actually wants to know — *what did this layer have to correct, and
would its author have agreed* — is only ever in them.
