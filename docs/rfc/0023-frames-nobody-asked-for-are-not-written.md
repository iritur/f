# RFC 0023: Frames nobody asked for are not written

- Status: accepted
- Date: 2026-09-01
- Affects: `kernel/src/mem.rs`; the boot-cost paragraphs of
  `docs/first-boot-outside-qemu.md`; the module documentation's claim that the
  free list has "no bootstrap problem" at any scale

## Decision

The frame allocator stops writing a link word into every frame at boot.
`add_region` still walks frames and still decides each one's fate with the
per-frame reserved-range filter, but it records its decisions as *runs* —
contiguous stretches of accepted frames, held in a small fixed array inside the
allocator — instead of threading every frame onto the free list. A frame's
link word is first written when the frame is actually *freed*; a frame that is
allocated straight off a run is handed over without the allocator ever having
touched it. The linked lists remain, unchanged, for everything that has been
handed out and given back.

## Context

The free list lives in the frames themselves — the first word of each free
frame holds the address of the next — which is why the allocator needs no
bitmap, no side array, and no allocator of its own. The cost was stated when
the design was written: initialising the list writes one word into every
frame. What was not understood was what that write costs on the machines this
kernel is now meeting.

On the first machine with real memory (16 GiB under VMware,
`docs/first-boot-outside-qemu.md`), the boot stalled for **two to three
minutes** between the address-space switch and the reclaim line — after the
quadratic reserved-range scan was already fixed, and in an optimised build.
The arithmetic identifies the culprit: roughly 150 seconds over 4.2 million
frames is ~36 µs per frame, which is not a DRAM write. It is a hypervisor
taking an extended-page-table violation, allocating a host page, and zeroing
it, once per frame — because writing one word into a page is touching it, and
touching every page of a 16 GiB guest at boot forces the host to commit all
16 GiB of it, serially, before the kernel has done anything. On bare metal
the same pass is merely a few hundred milliseconds of cache misses; under any
hypervisor that overcommits memory — which is all of them — it is the
dominant term of the entire boot, and it grows linearly with RAM forever.

Alternatives that were live:

- **Keep it and eat the cost.** Defensible on bare metal, and the measurement
  above is why it is not defensible in general: E0-P18's target machines
  include VMs, and a kernel whose boot time is set by the host's page-fault
  path is publishing the hypervisor's number as its own.
- **Interval subtraction.** Split each region against the reserved list once,
  by arithmetic. Rejected when the allocator was written, for a reason that
  still holds: the per-frame test cannot get the subtraction wrong, and
  interval arithmetic, done once, at four in the morning, can. This RFC keeps
  that rejection. The runs are produced *by the per-frame filter*, coalescing
  consecutive acceptances; no interval is ever subtracted from another. The
  one shortcut retained is the ceiling introduced after the first hardware
  boot: at or above every reserved end the filter's answer cannot be in doubt,
  so the remainder of a region is accepted as one run by construction rather
  than one frame at a time.
- **A bitmap or side table.** Reintroduces the sizing and bootstrap problems
  the intrusive list was chosen to avoid, to fix a cost that deferral fixes
  for free.

## Consequences

- Boot work in `add_region` drops from one write per frame to zero writes,
  and from one filter pass per frame to a filter pass only below the reserved
  ceiling. A hypervisor guest no longer pays for memory it has not used: host
  pages are committed as frames are genuinely allocated, not at boot.
- `alloc` prefers the dirty list, then a run, then the clean list; the dirty
  list keeps recycling hot frames, which in a VM also keeps allocation on
  already-committed pages. `free`, `scrub`, and the clean list are unchanged.
- The dirty count now includes never-issued frames — honestly, since their
  last owner is the firmware and their contents are unvouched-for. Every
  number the boot log and the state tree publish (`total`, `free`, clean and
  dirty at the hygiene line) is value-identical to what the eager
  initialisation produced.
- Allocation order changes: the eager list handed frames out last-added-first;
  runs are consumed ascending within a run, last run first. Every address in
  the boot log moves once. The log remains a fixture — two runs of one
  configuration still agree byte for byte, which is what E0-P02 claims; no
  golden log is stored, so nothing is regenerated.
- One address constraint survives the reordering and is stated so it cannot be
  lost: the AP trampoline loads the kernel's `CR3` as a **thirty-two-bit**
  value, so the kernel *root* — that one frame — must sit below 4 GiB. It
  does, structurally: `paging::build` allocates the root before `rebind`,
  when every run the allocator holds is below the 1 GiB identity limit. No
  other table is constrained: long-mode table entries are sixty-four-bit
  words, so the tables built after reclaim — the device window's, the
  on-ramp's — sit above 4 GiB on a large machine and are walked by arriving
  cores without complaint, and user-space roots are loaded with sixty-four-bit
  moves. An allocator change that let the *root* come from reclaimed memory
  would break the trampoline, and this bullet is where that is written down.
- The runs array is bounded (64 entries, one kibibyte inside the allocator).
  A memory map fragmented past it falls back, for the overflow only, to the
  old eager threading — slower, never wrong. The bound is a number, so the
  fallback is stated here rather than discovered.
- A frame allocated from a run has no link word to clear: `alloc` may now
  hand over a frame the allocator never wrote to, and `alloc_zeroed` zeroes
  it like any dirty frame. The hygiene guarantee is unchanged because it
  never rested on the link word — it rests on `alloc_zeroed` and `scrub`.

## What would reverse this

- A measurement showing run bookkeeping costing more than it saves on a real
  machine — which would mean memory maps fragmented far beyond the 64-run
  bound are common, and would argue for interval arithmetic proper rather
  than a return to eager writes.
- The buddy allocator (`docs/design/deadline-all-the-way-down.html` § 03),
  which replaces this allocator's structure wholesale; this RFC's deferral
  should be an input to that design — an allocator that touches memory nobody
  asked for has been measured, twice now, and lost both times.
- A defect traced to the double bookkeeping of runs beside lists — the class
  of bug the single-structure design was chosen to exclude. One such defect,
  root-caused, outweighs the boot time on any machine smaller than the one
  that motivated this.
