---
id: 0011
status: draft
originator: Dmitri Chudinov
todo:                # none yet; E3-00 says this epoch opens with an intent, and E5-D03 is the decision it leans on
---

# A picture at boot, and a display path with none of the framebuffer's debts

## Problem

Nothing this kernel does can be seen. The whole interface is a serial port at
38400 baud, and `docs/booting-on-hardware.md` says it plainly: with no serial
port you get a black screen and no way to tell a clean halt from a crash. Three
boots on a VMware machine were read through a cable. The one thing that has
ever reached a screen is a sixteen-by-sixteen pattern that `cargo xtask gpu`
photographs from outside the emulator, and it is a check rather than a
picture anyone looks at.

The obvious repair is the one every other kernel made: a framebuffer console
in the kernel. That is the thing I do not want. The framebuffer is a
forty-year-old shape and every one of its habits is a debt this tree has
already refused elsewhere: a single buffer the firmware chose, written by
privileged code holding a lock, read back from uncached memory to scroll,
torn because nothing owns it, flushed whole because nothing tracks what
changed, with no notion of when a frame reached the glass and no way to test
its output but a person looking. Linux has spent a decade retiring it and
still has not finished.

## Proposed outcome

- A person who boots F with a monitor attached and no serial cable sees the
  boot, from as early as the system can honestly show it, on a real machine
  and in the emulator.
- The screen is a *projection* of the boot log rather than a second output
  path: the same lines, the same order, the same bytes. Two runs of one
  configuration produce a pixel-identical screen, and the hash of the screen
  is checked the way the boot log already is.
- Whatever draws it runs outside the frame, restarts without losing the
  picture, and owns the client's pixels for exactly the interval a display is
  reading them — so nothing tears, because tearing is a buffer with two
  owners and this tree's types already forbid that.
- Work per frame is proportional to what changed, not to the size of the
  screen; copies per presented frame are counted, and on a device that can
  scan out of guest memory the count is zero.
- Every number the design promises for this — time to first pixel, copies per
  frame, bytes written per frame, crossings per frame — is a registered claim
  with a reproduction, pending until the machine exists, and never a sentence
  in a document.
- What is built here is the thin first slice of the interface epoch's
  presentation path and reuses its vocabulary, so the compositor lands on it
  rather than beside it.

## Affected users and systems

Whoever boots F on hardware — today, one person with a VMware guest and a
Threadripper. The display driver and its supervisor half. The frame, in one
place only: it must receive a firmware surface from the boot protocol, keep
the allocator off it, and route it to a component. The boot protocol decision
the real-hardware epoch already owes, because the emulator's kernel loader
cannot hand over a framebuffer and a real machine's firmware can. The
simulator's display model. `ring-scene-boot` parts II and III, which already
argue the presentation path this would be the first instance of, and which
should not have to change; if they do, that is a finding about this intent.

## Constraints

- No text renderer, font, or pixel path inside the frame. If bring-up on a
  machine without serial turns out to need something the frame draws before a
  component exists, that fallback is written down in advance with its cost to
  the unsafe count, the way the real-hardware epoch already treats the
  graphics shim.
- No floating point anywhere in the pipeline. Layout and coverage are fixed
  point with the scale in the name.
- The compositor, the scene graph, GPU rasterisation and the semantic
  vocabulary stay in E3. This is a text projection of one stream and must
  not grow a window system.
- The existing datapath check keeps passing, and the observation stays
  outside the machine.
- Anything imported — a bootloader binary, a font — sits inside the licence
  boundary or is refused.

## Open questions

- Does the pinned emulator's kernel loader honour a multiboot video-mode
  request at all, or only warn? The answer decides whether the emulated path
  and the hardware path share a boot protocol or split.
- How early is early enough? The frame's own log starts before any component
  can exist. Is a picture that begins a few milliseconds in and replays the
  log from the first line acceptable, or does bring-up need the frame to
  draw?
- A 4K surface in the display's format is tens of mebibytes and the mode is
  not known until boot. What does a manifest that has to size a component's
  memory before boot declare — a maximum mode, or a component that draws in
  a window of whatever it is given?
- Neither backend this tree can reach today reports when a frame hit the
  glass. What does the completion's timestamp mean until a device that has a
  vertical blank exists, and is a claim about it honest to register now?
- The safe accessor a component reaches device memory through is per-word
  and bounds-checked. Is that fast enough for a damage-sized blit into
  write-combining memory, or does it owe a bulk copy first?
