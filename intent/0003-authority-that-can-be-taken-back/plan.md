---
id: 0003
status: done
spec: ./spec.md
---

# Plan: the capability table, and the negative suite

One change, one pull request, and the reason is the same as intent 0002's: no
part of it is observable until the last part lands. A table nothing holds, a
handle nothing presents, a refusal nobody earns. Splitting it would produce two
pull requests, neither of which can say whether it works.

## Files

```
abi/src/cap.rs                       NEW: the wire half — the handle packing,
                                     the six types, the six rights, and the
                                     tests for all three
abi/src/lib.rs                       the module, and two authority codes plus
                                     two argument codes the frame needs to name
                                     its refusals
kernel/src/cap.rs                    NEW: the table, the derivation tree, the
                                     five properties as a function over a
                                     trait, and the five tables broken on
                                     purpose that prove the function can fail
kernel/src/arch/x86_64/paging.rs     `UserPage::ReadOnly`, because a read-only
                                     grant must not arrive executable;
                                     `map_user_live`, which maps into a running
                                     address space and allocates nothing;
                                     `BuildError::NoTable`; `Features::NONE`
kernel/src/process.rs                the grants, the four calls, the tally the
                                     frame keeps of what it answered, the seven
                                     capability provocations and exactly what
                                     each must earn
kernel/src/arch/x86_64/probe.rs      the preamble every process runs, the seven
                                     escape blocks, and the constants the
                                     program and the frame must not disagree
                                     about
kernel/src/main.rs                   the property suite at boot, and the
                                     capability line in the process report
xtask/src/main.rs                    `cargo xtask cap [kind]`, its help, and a
                                     fix to `lint-percpu`, which read
                                     `&'static mut T` as a mutable global
.github/workflows/ci.yml             the seven escapes join the gate
docs/rfc/0015-capabilities-at-the-door.md
                                     NEW: why four calls appear behind a door
                                     that exists to stop calls appearing
intent/0003-…/                       NEW: this intent, its spec and this plan
TODO.md                              E0-B11 done, naming this intent; E0-P08
                                     updated with what it still owes
CLAUDE.md                            one line: the new command
```

## Order

Whatever cannot be observed until something else exists, last.

1. `abi/src/cap.rs`. The handle packing first, because everything else is
   written against it and because it is the only part with host tests — a
   packing that is wrong is cheapest to find here.
2. `kernel/src/cap.rs`, the table. Still nothing holds one, so nothing can tell
   whether it works; that is what step 3 is for and why it comes immediately.
3. The properties and the flawed tables, in the same file, run from `main`. This
   is the first step that produces evidence, and it produces it about the part
   with the most logic in it. It also found the first real mistake: a fixture
   that broke two things at once — masking the index *and* collapsing the
   generation check — was caught by the wrong property, which is exactly what
   the wrong-property arm exists to report.
4. `paging.rs`. The read-only mapping and the live one, before there is anything
   to map. `map_user_live` allocating nothing is what keeps step 6's free-count
   assertion meaningful.
5. `process.rs`. The grants, the calls, the tally, the expectations.
6. `probe.rs`. The preamble and the seven escapes. Last of the kernel work
   because it is the only part that cannot be checked except by running it.
7. `xtask`, then CI.
8. The documents.

## What this expects to be wrong about

Named in advance, so that being wrong is a finding rather than a surprise.

- **The exact counts.** Every expectation is an exact tally, and the first run of
  each escape is where the arithmetic meets the assembly. A count that is off by
  one is a real answer about what the frame did, not a number to adjust until it
  matches.
- **Register discipline in the probe.** Only six registers survive a call, the
  program needs four of them across the whole run, and the sweep needs two more
  for its loop. If something is destroyed across a `syscall` it will look like a
  capability failure rather than like what it is.
- **The order the checks are in.** Authority before argument. A refusal that
  arrives as an argument error where an authority error was expected means the
  order is wrong, and it fails the tally rather than passing quietly.
