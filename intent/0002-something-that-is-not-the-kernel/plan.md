---
id: 0002
status: done
spec: ./spec.md
---

# Plan: user page tables, the ring-3 transition, and a door back

One change, in one pull request, and the reason it is not split is the reverse
of intent 0001's. There the two halves failed differently — one against hardware
and one against arithmetic — and reviewing them together meant reviewing
neither. Here every piece is unobservable until the last one lands: an address
space nothing enters, a transition with nowhere to go, a system call entry no
process can reach. A pull request that added any one of them would add code with
no way to tell whether it worked.

## Files

```
kernel/src/arch/x86_64/gdt.rs        the three ring-3 descriptors, in the order
                                     `sysret` requires; `IA32_STAR`'s value; and
                                     the pointer to this core's ring-0 stack
                                     slot, handed out rather than written
kernel/src/arch/x86_64/paging.rs     `USER`; `UserSpace`, whose upper half is
                                     the kernel's and whose frames are tracked
                                     so they can be given back; `map_user` and a
                                     walk of its own, because a user table entry
                                     carries a bit the kernel's never does;
                                     `switch`
kernel/src/arch/x86_64/ring3.rs      NEW: the per-core entry block, the four
                                     model-specific registers, the `syscall`
                                     stub, the hand-built interrupt frame that
                                     leaves for ring 3, and the two ways back
kernel/src/arch/x86_64/probe.rs      NEW: the program. Sixty-odd instructions of
                                     position-independent assembly in `.rodata`,
                                     and the seven violations it can commit
kernel/src/process.rs                NEW: what a process is — build, run, end,
                                     give the memory back, and judge whether the
                                     provocation provoked
kernel/src/arch/x86_64/idt.rs        one branch: a fault carrying a ring-3 code
                                     selector ends a process instead of the
                                     machine, and a tick carrying one is counted
kernel/src/arch/x86_64/apic.rs       `run` becomes `start`/`wait`/`stop`, because
                                     what happens inside a timer window is now a
                                     process rather than a wait
kernel/src/arch/x86_64/mod.rs        the module list, and the doc line that says
                                     what M3 added
kernel/src/main.rs                   `run_timer` splits into `calibrate` and
                                     `timed_window`; the process runs inside the
                                     window; the PCID line stops naming a
                                     milestone that has arrived
xtask/src/main.rs                    `cargo xtask user [kind]`, and its help
.github/workflows/ci.yml             the seven provocations join the gate
docs/rfc/0014-the-syscall-door.md    NEW: what the entry is for, and the rule
                                     for adding to it
intent/0002-…/                       NEW: this intent, its spec and this plan
TODO.md                              E0-B09 done, naming this intent
CLAUDE.md                            one line: the new command
```

## Order

Whatever cannot be observed until something else exists, last.

1. `gdt.rs`. Nothing can be at ring 3 without descriptors for it, and the
   `sysret` ordering constraint is the one mistake in this change that produces
   no fault — it returns to ring 3 through whatever descriptor happened to be at
   that offset.
2. `paging.rs`. The address space, before there is anything to put in it. The
   user bit at every level is the other silent mistake: set only in the leaf, it
   looks right in the table dump and the page is invisible from ring 3.
3. `ring3.rs`. The transition and the entry, written together because the stack
   argument is one argument covering both.
4. `probe.rs`. The program, which is the first thing that can tell whether any
   of the three above works.
5. `process.rs`. Life, death and the memory audit.
6. `idt.rs`. The branch that makes a ring-3 fault survivable — after there is
   something that can take one, so that the first run of it is a real one.
7. `apic.rs`, then `main.rs`. The window the process runs inside.
8. `xtask`, CI, RFC 0014, the intent, `TODO.md`.

## How each step is checked

Steps 1 and 2 are checked by `process::self_test`, which runs at every boot and
asserts the two things that produce no fault when wrong: that `IA32_STAR` names
the segments `sysret` would actually load, and that the layout is three
consecutive pages inside one page table's reach.

Steps 3 to 7 are checked by the boot itself, which is why they are in this
order: the first process to run is the test of all of them, and it either
announces itself or the log says it did not.

Step 8 is checked by `cargo xtask user`, which is the suite: seven boots, six
faults that must happen and one that must not.

## What this plan does not do

It does not touch anything above the frame, and it does not add a dependency.
`user/init` is untouched: making it the process that runs is E0-B10, and doing
it here would mean building a loader inside a change that is already about three
transitions and an address space.
