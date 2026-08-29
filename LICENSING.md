# Licensing

The licence boundary and the isolation boundary are the same boundary.
This is a structural decision, not a paperwork one. See `docs/design/fast-path.html`
section 14.

## The rule

| Tree | Licence | Why |
|---|---|---|
| `abi/`, `env/`, `ring/`, `kernel/`, `user/`, `xtask/` | Apache-2.0 OR MIT | The research must be reusable. Everything written for this project is permissively licensed so results can be lifted by anyone. |
| `third_party/<name>/` | Whatever that source requires | Imported driver source and its shim. Delimited, never mixed into the permissive tree. |

## Why this is clean here and messy elsewhere

Imported Linux driver source is GPL-2.0, and a component built from it is a
derivative work. FreeBSD manages the same problem in-kernel and rests the
separation on a linking argument.

F does not have to make that argument. Imported drivers run as isolated
components: **separate address space, no shared symbols, communication only over
a ring**. That is a far stronger separation than linking, and it lands exactly
where the licence boundary needs to be.

Consequence: an imported driver is never linked into the frame, even where the
confinement carries a measurable cost. Safety was already one reason. This is a
second, independent one.

## Rules that follow

1. No file under `third_party/` may be `use`d from the permissive tree. The only
   permitted coupling is the ring protocol defined in `abi/`.
2. Every file in the permissive tree carries `SPDX-License-Identifier: Apache-2.0 OR MIT`.
3. Every imported tree carries its own `LICENSE` and a `PROVENANCE.md` recording
   upstream URL, commit hash, and the date imported.
4. `cargo xtask lint-licensing` enforces 1 and 2 in CI.
