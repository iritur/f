# third_party

Imported source and imported data. **Nothing in the permissive tree may `use`
anything here.** The only permitted coupling to imported *source* is the ring
protocol defined in `abi/`. Imported *data* — files no compiler in this
workspace reads — has a second, and it is narrow: a table generated from it by
a command and committed, and a named host test that opens a corpus file by path.
Both are rows in `xtask/src/main.rs` (`DERIVED_DATA`, `IMPORT_READERS`). RFC 0114.

Each import gets its own directory containing:

- `LICENSE` — the terms it arrived under
- `PROVENANCE.md` — upstream URL, commit hash (for data, the version), date
  imported, what was changed; and for data, every file with its byte count and
  SHA-256
- the source or the data itself, verbatim

`cargo xtask lint-licensing` enforces the isolation, and reads each
`PROVENANCE.md` against the bytes it describes. See `LICENSING.md` for why the
boundary is drawn here and why it is stronger than the equivalent argument
FreeBSD has to make.

`unicode/` is the first import: Unicode 17.0.0 data files, imported 2026-09-25.
RFC 0003 sets out what else arrives and when: graphics and wireless imported,
storage, network, audio, accelerators and the input path written.
