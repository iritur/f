# RFC 0129: The compositor that is restarted is the one that timed out

- Status: accepted
- Date: 2026-09-25
- Affects: `kernel/src/component.rs` (`Served` replaces `Reading`;
  `Datapath` gains `prepare` and `ended`; `Wired` carries the occupant's tree,
  its physical page, its heap and its epoch; the timeout section serves the
  place's own occupant and copies the two words through the frame's root mount
  of its tree; `Failure::Unserved`), `kernel/src/compositor.rs` (`Placed`, the
  client of a compositor served in its place; `demonstrate` keeps the two
  halves that stand nothing up and refuses the other three by name;
  `Report::served` and `Report::served_held`), `kernel/src/main.rs`
  (`compositor_boot` hands the lifecycle a client instead of a reading;
  `liveness_of` is gone), `user/compositor/src/` (`reported::EPOCH`, the epoch
  the component read off its own control ring), `user/compositor/manifest.toml`
  (a `board` and a `data` need), `xtask/src/main.rs` (`identity_held`), RFC
  0126, and `TODO.md` task `E3-B05e`.

## Decision

**A compositor that stands up is served from its place, and the reading a
supervisor judges is copied out of that occupant's own tree, so the occupant it
stops and restarts is the one that published.** Three parts:

- **The place's own occupant is handed a core and a client.** The three
  serving halves of `cargo xtask compositor` — `serve`, `starved`, `wake` — are
  run by `component::demonstrate` against the compositor place's occupant with
  `compositor::Placed` as the `Datapath`, exactly as the block place's occupant
  is served by `blk::Placed`. `kernel/src/compositor.rs` no longer stands a
  compositor up beside its place, and a serving half asked of the path that
  stands nothing up is `Trouble::Placed`, by name.
- **The two words are copied through the frame's own mount.** The frame follows
  the mount word its root holds for the place — which `mount` wrote at the
  occupant's spawn and `tear_down` zeroes at its end — and copies waits
  outstanding and frames abandoned out of the tree it names, by the node ids the
  client supplies. It reads neither; RFC 0123's reversal is unchanged.
- **Which instance, on both sides.** The component reads the `epoch` the frame
  wrote into its control ring's header when it spawned *that* instance, and
  reports it plus one (`reported::EPOCH`); the kernel's verdict requires it to be
  the epoch of the occupant it served. The lifecycle prints the occupant's epoch
  and its tree's physical page at the copy and at the teardown, and `cargo
  xtask compositor` requires the served line, the liveness line and the timeout
  line to name one epoch and one page, and the refill to be the next epoch.

## Context

RFC 0126 closed every link of *a boot shows a compositor restarted for a
timeout* on real readings and recorded one narrowing: the reading was the
manifest's. `kernel/src/compositor.rs` stood a compositor up with
`process::prepare_server`, outside any place, read two words off its tree and
handed them to the lifecycle, which put them on the row of the compositor
place's occupant — an instance that had never run — and then stopped and
restarted that one. `E3-B05e`'s exit names one compositor, and there were two.

**The reason it ran beside its place was not the one its file gave.** The file
header said there was no other client: the supervisor did not hand a place's
occupant a core and a peer. That had stopped being true when `component::
Datapath` gave the block place's occupant a client inside the lifecycle. What
was left was a declaration: `user/compositor/manifest.toml` declared a `heap`
and a `self` and nothing else, and `spawn` maps a routing page and ring memory
only for a manifest that declares a `board` and a `data` need. A spawned
compositor had nowhere to learn where its rings were, and its serving life would
have ended `NO_ROUTING` before it looked. `user/virtio-blk` hit the same wall at
`E1-B05` and answered it by declaring both; the compositor does the same.

Three alternatives were live.

*Map the two pages into the occupant without a declaration*, out of frames the
frame allocates, as the swap window once was. Rejected: an account that does
not pay for what its occupant is given is a quota that does not bound it, and a
manifest that does not say what its component is made of is a manifest `lint-
manifests` checks arithmetic against for nothing.

*Keep the beside instance and carry its identity onto the row.* It would make
the gap observable and not close it: the instance stopped would still not be
the instance that published.

*Serve the compositor from its place and fall back to the beside instance when
the manifest lacks the two needs.* Rejected because the fallback is the defect:
a merged tree without the rows would pass every check over two compositors.
The lifecycle refuses such a place with `Failure::Unserved`, and the boot log
prints a line beginning `unserved      place compositor declares no` and naming
which of the two is absent.

## Consequences

**What it makes easy.** The chain `E3-B05e` asks for is one instance end to
end: `cargo xtask compositor serve` shows the occupant at epoch 0 serving its
client, its tree at one page, the frame copying `waits 1, abandoned 1` through
the root's mount of that page, the supervisor naming a timeout, the occupant at
epoch 0 with that page ended and unmounted, and the place refilled at epoch 1.

**Why an epoch and a page, and not one of them.** An epoch alone repeats: the
beside compositor described its rings at epoch zero, and so does every place's
first occupant, so an epoch check would have passed over exactly RFC 0126's
arrangement. A page alone repeats too: a place's account refunds from the top of
its watermark and the next spawn takes the same frames, so a refilled
occupant's tree can land on the page its predecessor's did. Together they name
one instance for the length of a boot.

**The narrowings, said plainly.**

- *Stopped* is *ended and torn down under the named cause*. The occupant's core
  comes back before the supervisor runs — it serves its client, is told to stop
  by that client, and ends — because a place's occupant in this frame runs in
  bounded runs on a core it is lent, and the store's is the same. The stop is
  performed on the instance whose tree the words came from; it does not
  interrupt a core still inside it.
- *The starved half describes two pages and maps the manifest's heap.* A place
  maps the heap its manifest declares; the prologue the component reads is the
  frame's, and it says two pages. The half proves the component refuses on the
  size it is told.
- *The wake half's doorbell counts are differences*, because the worker core is
  shared with the supervisor's consultations. Nothing before the compositor
  halts that core in this build, so the subtraction is not exercised: a
  mutation that dropped it left the half green.
- *`cargo xtask input` still stands a compositor up beside any place*, in
  `kernel/src/input.rs`. It carries no liveness reading and no supervisor judges
  it, so it is not this RFC's subject; it is the next instance of the same
  shape.

**What it costs.** The compositor's account pays two more pages per occupant
(54 frames where it was 52, of 64). `Datapath` has two more methods, both
defaulted, so the block client is unchanged. The compositor verdict is printed
after the component lifecycle rather than before it.

## What would reverse this

**An occupant that holds its core across a consultation** — a second worker, or
a scheduler. Then a stop for a timeout is a kill of a running instance, and
`Killing` is how; the sentence *the core came back before the supervisor ran*
stops being true and the first narrowing goes.

**An identity the frame mints per instance.** An instance number no two
instances in a boot share, written where the component can read it, replaces
the pair: `reported::EPOCH` carries that, and the page stops being needed.

**A manifest that serves a client and declares no board or data**, if the frame
ever gains another way to tell an occupant where its rings are — a capability
the occupant adopts by calling for it. Then the declaration stops being the
reason and the refusal in `component::demonstrate` goes with it.

**A second place served in place.** `Served` names one place; a boot that serves
two would want a list, and the identity check in `xtask` would want to be keyed
by label rather than by the compositor's.
