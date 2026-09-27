# third_party/harfrust — provenance

HarfRust, a port of HarfBuzz's shaping to Rust, imported **as source** under RFC
0082: the shaper is imported, built into a component image of its own, and
reached from the permissive tree over a ring and by no other route. What it does
here is narrow and RFC 0082 names it: glyphs and positions for one run of text in
one face at one size under one feature set. Everything around that — bidi, line
breaking, the cache, metrics — is written in `text/`.

| Field | Value |
|---|---|
| Kind | source |
| Upstream | https://github.com/harfbuzz/harfrust, published as the crate `harfrust` on crates.io |
| Commit | `fbff7e563b4e512686c9e627d91a147ba4ee2402`, tag `0.13.3`, released 2026-08-25 |
| Imported | 2026-09-26 |
| Retrieved with | `cargo vendor --versioned-dirs` over a manifest asking for `harfrust = "=0.13.3"` with `default-features = false, features = ["libm"]`, run in the `f-dev` container |
| Changed | nothing |

`Changed: nothing` is checkable rather than asserted: every crate under
`vendor/` carries the `.cargo-checksum.json` `cargo vendor` wrote, and cargo
refuses to build a vendored crate whose files do not hash to it. The package
checksums below are crates.io's, copied from `Cargo.lock`, which is this import's
resolution and is kept beside this file so the set cannot drift on a rebuild.

## What was imported, and why each crate is here

The build asked for is the one with no `std`: HarfRust's `default` feature turns
`std` on, and the component this lands in has no operating system under it. `libm`
supplies the floating-point functions `std` would have. Nothing else was asked for;
every crate below is one Cargo resolved from that single request.

| Crate | Version | Licence | crates.io checksum (SHA-256) | Why it is here |
|---|---|---|---|---|
| `harfrust` | 0.13.3 | MIT | `948d0741125ba89cd3e1c23e5642415b6ade7e1d29d67ba25fb925b533e989d6` | the shaper |
| `read-fonts` | 0.43.3 | MIT OR Apache-2.0 | `005c8acf251756c478b0bf402885bfd88a1476020c4c7e6060edc1aa68da38ea` | fontations' parser for the font's tables, which HarfRust reads through |
| `font-types` | 0.12.5 | MIT OR Apache-2.0 | `b8eb065f3251655b3c90e22e5e363f310fc5332fb3402e37bbc94752283248f6` | the scalar types those tables are made of |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | `95832e849adfb21180ccb6826a99da14e5d266ae5c2e668e1602cf234f153797` | casts between byte slices and plain data |
| `bytemuck_derive` | 1.12.1 | Zlib OR Apache-2.0 OR MIT | `6a1f896587b6f2c069c73d2f0913e2d590c3990285cd2f0b6aa02b786b4c679c` | a procedural macro for `bytemuck`, run on the host at build time |
| `syn` | 3.0.6 | MIT OR Apache-2.0 | `8593e8e72159ed2257d083c7a454a85cbf854f37a0966d8d483aff8c8a3ebcee` | the Rust parser `bytemuck_derive` is written with; host only |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | `1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001` | the same; host only |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | `985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9` | the same; host only |
| `unicode-ident` | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | `d245f478577f809a851594d02313b640fb437e0bb33866753cff937863096954` | the same; host only |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 | `9f7c3e4beb33f85d45ae3e3a1792185706c8e16d043238c593331cc7cd313b50` | lazily built shared tables, in its `race` form, which needs no lock |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | `3ded4057c258ba199e2d26386d3af3780957ecaee6c4ef4041c6b4b8b97c0b06` | flag sets |
| `smallvec` | 1.16.2 | MIT OR Apache-2.0 | `f9395f0f0eee849a9b707b2f06bb92a6a422090e2123bb2ef8e87a0e61892a8e` | short buffers without a heap allocation |
| `core_maths` | 0.1.1 | MIT | `77745e017f5edba1a9c1d854f6f3a52dac8a12dd5af5d2f54aecf61e43d80d30` | float methods on `core` types, forwarded to `libm` |
| `libm` | 0.2.16 | MIT | `b6d2cec3eae94f9f509c767b45932f1ada8350c4bdb85af2fcab4a3c14807981` | the maths library `std` would otherwise supply |

Fourteen crates, 833 files. `LICENSE` beside this file is HarfRust's, the terms the
import is named for; each crate's own licence files are in its directory, and every
licence above is one `deny.toml` would admit.

## Three things a reader should know before trusting this

- **Four of the fourteen run on the build machine, not in the component.**
  `bytemuck_derive` is a procedural macro, and `syn`, `quote`, `proc-macro2` and
  `unicode-ident` exist only to build it. They execute on the host at compile
  time and are linked into nothing that runs on F. RFC 0082's build-surface rules
  were written for the permissive tree; whether a procedural macro inside an
  import is acceptable is `E3-B03b0`'s to settle and record.
- **HarfRust computes in floating point.** RFC 0082 permits that inside the
  component, on the condition that only fixed-point integers cross the ring and
  that a shaping corpus shaped on both architectures is what would catch a
  position moving with anything but its inputs.
- **Two files carry carriage returns**, as upstream published them.
  `.gitattributes` marks `third_party/**` `-text`, so git never rewrites them.

## Updating

A version bump is a re-import: run the same `cargo vendor` request at the new
version into an empty directory, replace this one whole, rewrite the tables above
from the new `Cargo.lock`, and review what changed. RFC 0082 calls a re-import a
review, and `docs/TECHNICAL-DEBT.md` records that Skrifa should arrive in the
same bump, so the component carries one `read-fonts` and not two.
