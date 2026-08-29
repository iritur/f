# third_party

Imported source. **Nothing in the permissive tree may `use` anything here.**
The only permitted coupling is the ring protocol defined in `abi/`.

Each import gets its own directory containing:

- `LICENSE` — the terms that source arrived under
- `PROVENANCE.md` — upstream URL, commit hash, date imported, what was changed
- the source itself

`cargo xtask lint-licensing` enforces the isolation. See `LICENSING.md` for why
the boundary is drawn here and why it is stronger than the equivalent argument
FreeBSD has to make.

Empty at M0. RFC 0003 sets out what arrives and when: graphics and wireless
imported, storage, network, audio, accelerators and the input path written.
