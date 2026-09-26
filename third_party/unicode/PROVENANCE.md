# third_party/unicode — provenance

Unicode Character Database data files, imported **as data** under RFC 0114's
third category: files no compiler in this workspace reads. Two routes lead out
of this directory and no third — a property table generated into `text/src/` by
`cargo xtask unicode`, and a conformance corpus opened by path from a named host
test. `LICENSING.md` says what each route owes; `LICENSE` beside this file is
the terms the files arrived under.

The two tables below are read by `cargo xtask lint-licensing`, which is what
makes this file a record rather than a description: every file in this
directory must have a row, every row must name a file that is here, and each
file's byte count and SHA-256 must be the ones written down. A file that is not
byte-identical to upstream is not the specification's data any more, and the
check says so before anything is generated from it or tested against it.

| Field | Value |
|---|---|
| Kind | data |
| Upstream | https://www.unicode.org/Public/17.0.0/ucd/ |
| Version | 17.0.0 |
| Imported | 2026-09-25 |
| Retrieved with | `curl.exe -fsSL`, one request per file, from the URL in the *From* column below |
| Changed | nothing |

`Version` is what the files state about themselves, not what anybody
remembers: every `.txt` file here whose first line is `# <Name>-<X.Y.Z>.txt`
says `17.0.0` there, and the check reads that line and compares it to this row.
`emoji/emoji-data.txt` says `# Version: 17.0` further down its header instead,
and `LICENSE` states no version; neither is compared, and the check prints how
many files it compared so the count is visible rather than assumed.

| File | Bytes | SHA-256 | From |
|---|---|---|---|
| `BidiBrackets.txt` | 8891 | `dadbaf38a0d0246e5b805bf8725cb81b7c621f93d030595635f5ba2c2f179428` | https://www.unicode.org/Public/17.0.0/ucd/BidiBrackets.txt |
| `BidiCharacterTest.txt` | 6880771 | `a3e6e905ab5afbe318a96df5401d0372a04cd73ef139ab5e3cf0ae241c255488` | https://www.unicode.org/Public/17.0.0/ucd/BidiCharacterTest.txt |
| `BidiMirroring.txt` | 26827 | `a2f16fb873ab4fcdf3221cb1a8a85a134ddd6ed03603181823ff5206af3741ce` | https://www.unicode.org/Public/17.0.0/ucd/BidiMirroring.txt |
| `BidiTest.txt` | 7959988 | `888bdfc8090652272d1f859cdb00ae659e2dc6c26740be61ef1d03998a687620` | https://www.unicode.org/Public/17.0.0/ucd/BidiTest.txt |
| `DerivedCoreProperties.txt` | 1134783 | `24c7fed1195c482faaefd5c1e7eb821c5ee1fb6de07ecdbaa64b56a99da22c08` | https://www.unicode.org/Public/17.0.0/ucd/DerivedCoreProperties.txt |
| `EastAsianWidth.txt` | 201595 | `ea7ce50f3444a050333448dffef1cadd9325af55cbb764b4a2280faf52170a33` | https://www.unicode.org/Public/17.0.0/ucd/EastAsianWidth.txt |
| `LICENSE` | 1995 | `e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96` | https://www.unicode.org/license.txt |
| `LineBreak.txt` | 263180 | `e6a18fa91f8f6a6f8e534b1d3f128c21ada45bfe152eb6b1bcc5e15fd8ac92e6` | https://www.unicode.org/Public/17.0.0/ucd/LineBreak.txt |
| `auxiliary/GraphemeBreakProperty.txt` | 99377 | `d6b51d1d2ae5c33b451b7ed994b48f1f4dc62b2272a5831e7fd418514a6bae89` | https://www.unicode.org/Public/17.0.0/ucd/auxiliary/GraphemeBreakProperty.txt |
| `auxiliary/GraphemeBreakTest.txt` | 126570 | `e2d134d2c52919bace503ebb6a551c1855fe1a1faec18478c78fff254a1793ec` | https://www.unicode.org/Public/17.0.0/ucd/auxiliary/GraphemeBreakTest.txt |
| `auxiliary/LineBreakTest.txt` | 3166819 | `e69884e0dde6a8724873f885d68c52dc14518abf9ae4ca9e2283b8773db3b752` | https://www.unicode.org/Public/17.0.0/ucd/auxiliary/LineBreakTest.txt |
| `emoji/emoji-data.txt` | 107324 | `2cb2bb9455cda83e8481541ecf5b6dfda66a3bb89efa3fa7c5297eccf607b72b` | https://www.unicode.org/Public/17.0.0/ucd/emoji/emoji-data.txt |
| `extracted/DerivedBidiClass.txt` | 173433 | `4867b4b7f0731ed1bfcd34cc6251211ff1542541fce0734b6fbda139ee80b3a4` | https://www.unicode.org/Public/17.0.0/ucd/extracted/DerivedBidiClass.txt |
| `extracted/DerivedGeneralCategory.txt` | 277514 | `d62e5bab70ca74f099343f71224fa051cb1fdd61a1ab45c0488c44cfc0b6102e` | https://www.unicode.org/Public/17.0.0/ucd/extracted/DerivedGeneralCategory.txt |

Fourteen files, 20,429,067 bytes. The first twelve were imported on
2026-09-25 for `E3-B03e0`, and each byte count and hash was measured twice,
independently — by the coordinator on the day of import and by that line's
builder — and the two readings agreed. `DerivedCoreProperties.txt` and
`extracted/DerivedGeneralCategory.txt` were added the same day for `E3-B03f`, by
the same command, and hashed by the coordinator; `lint-licensing` is the second
reading.

## Why each file is here

RFC 0114's reversal condition is *the third thing under this directory that is
not a UAX property table*, so each file says which rule selected it.

- **Generated today**, into `text/src/`: `extracted/DerivedBidiClass.txt`
  (`Bidi_Class`, UAX #9) into `text/src/bidi_class.rs`, and `BidiBrackets.txt`
  (the pairs rule N0 needs) into `text/src/bidi_brackets.rs`.
- **Corpus**, opened by a host test and never generated, compiled or embedded:
  `BidiTest.txt` and `BidiCharacterTest.txt` for `E3-B03e`,
  `auxiliary/LineBreakTest.txt` and `auxiliary/GraphemeBreakTest.txt` for
  `E3-B03f`.
- **Held for `E3-B03f`**, each a property UAX #14 or UAX #29 reads in its own
  rules: `LineBreak.txt`, `auxiliary/GraphemeBreakProperty.txt`,
  `EastAsianWidth.txt` (UAX #14 reads East_Asian_Width) and
  `emoji/emoji-data.txt` (both read Extended_Pictographic).
- **Added for `E3-B03f`**, each a property a rule of that line's level reads
  and nothing else here holds: `DerivedCoreProperties.txt` for
  `Indic_Conjunct_Break` (UAX #29 rule GB9c), and
  `extracted/DerivedGeneralCategory.txt` for `General_Category` (UAX #14 rule
  LB1, which resolves `SA` to `CM` or `AL` by it).
- **Held for `E3-B03e`'s rule L4**: `BidiMirroring.txt`, the mirrored glyph a
  reordered bracket is drawn with.
- `LICENSE`: the Unicode License V3, as `https://www.unicode.org/license.txt`
  served it on the day of import.

## Updating

A version bump is a regeneration, a re-hash and a review rather than an edit:
replace the files, rewrite the tables above from the files themselves, change
the hashes in `DERIVED_DATA` in `xtask/src/main.rs`, and run
`cargo xtask unicode`. `cargo xtask lint-licensing` is red at every step until
all of them agree, which is the point.
