# third_party/inter — provenance

One face, imported **as data** so that `E3-B03b0` has something to shape: the
exit asks for one run of text through the `shape` protocol, and a shaper with no
face shapes nothing. No compiler in this workspace reads it. It reaches a
component the way every face does, by content hash out of the blob store
(`E3-B03b`), and the shaper reads its tables behind the ring.

| Field | Value |
|---|---|
| Kind | data |
| Upstream | https://github.com/rsms/inter, release `v4.1` |
| Version | 4.001 |
| Commit | `e3a3d4c57d5ecc01453a575621882a384c1995a3`, the commit tag `v4.1` names, released 2024-11-16 |
| Imported | 2026-09-26 |
| Retrieved with | `curl.exe -fsSL` of the release's one asset, `https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip`, then two files extracted from it with nothing else taken |
| Changed | nothing |

`Version` is what the face states about itself: `head.fontRevision` is 4.001
and its `name` record 5 reads *Version 4.001;git-9221beed3*. The release is
tagged 4.1; the face inside it calls itself 4.001, and this record uses the
face's word.

**Upstream publishes no checksum for the release**, so the archive's hash below
is this tree's own measurement, taken on the day, and a later fetch of the same
URL is checked against it rather than trusted.

| Archive | Bytes | SHA-256 |
|---|---|---|
| `Inter-4.1.zip` | 33707794 | `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e` |

| File | Bytes | SHA-256 | From |
|---|---|---|---|
| `LICENSE` | 4380 | `262481e844521b326f5ecd053e59b98c8b2da78c8ee1bdbb6e8174305e54935a` | `LICENSE.txt` in the archive, byte-identical to `LICENSE.txt` at tag `v4.1` |
| `Inter-Regular.ttf` | 411640 | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` | `extras/ttf/Inter-Regular.ttf` in the archive |

Two files, 416,020 bytes, hashed by the coordinator on the day of import.

## Why this face, and what it is not

- **Why Inter.** It is designed for text on screens at interface sizes, which is
  what this system draws. It is under the SIL Open Font License 1.1, which
  permits redistribution with the licence beside it. Chosen by the project's
  owner on 2026-09-26, in place of Lato, before either was committed.
- **Why the static Regular and not `InterVariable.ttf`.** The release's main
  font is variable, with weight and optical-size axes. The first run through
  the shaper should exercise `GSUB` and `GPOS` without also exercising a
  variation instance, and a static face has no `fvar`. The variable face is the
  later import, on the day a weight is chosen at run time.
- **What it is not: a font corpus.** It covers Latin, Greek and Cyrillic and
  carries no Hebrew, Arabic, Indic or CJK glyphs, so most scripts in
  `text/src/corpus.rs` cannot be shaped with it. A corpus of faces chosen one
  per script, argued the way the sample texts are, is later work and a separate
  import.
- **What it breaks on arrival.** Its `maxp.numGlyphs` is 2,937.
  `text/src/face.rs`'s `GLYPHS_MAX` is 1,024, and that module names a real face
  arriving as the day its own format gives way to the font's header.

## Updating

Replace a file only with the same file from a newer release: fetch the archive,
record its hash, extract the same two paths, re-hash them, and rewrite the tables
above. The OFL's Reserved Font Name clause means a changed face may not be called
Inter, which is one more reason `Changed` stays *nothing*.
