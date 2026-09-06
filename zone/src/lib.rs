// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The zoned mapping: sequential fill, seal, copy-forward, reset — and the
//! collector that decides which zone is worth the trouble.
//!
//! # Where this sits
//!
//! Between `f_blob::store::Store` and the device. The store above writes blocks
//! to a flat address space and has no idea a zone exists; the device below
//! accepts an append at a write pointer and refuses everything else. This crate
//! is the translation, and it is a separate crate rather than a module of
//! `blob/` because a store that knew about zones would be a second thing that
//! could disagree with this one about an offset — which `blob/src/store.rs`
//! declines to be in its own first paragraph.
//!
//! ```text
//!   f_blob::store::Store          blobs and objects, a flat block address space
//!   ── map::ZoneMap ──            logical → physical, fill, seal, copy, reset
//!   ── device::Zoned ──           ZONE_APPEND, ZONE_FINISH, ZONE_RESET, ZONE_REPORT
//!      device::ZonedMemory        the emulation these host tests run against
//! ```
//!
//! # What each module owns
//!
//! - [`device`] — RFC 0060's four zone opcodes as a trait, and a model that
//!   refuses what a sequential-write-required device refuses.
//! - [`map`] — the mapping, and the per-zone state RFC 0059's predicates range
//!   over.
//! - [`roots`] — `R = P ∪ T`, and the add-then-drop handover that is the one
//!   ordering a publish may not get wrong.
//! - [`mark`] — `B`, and the walk that fills it.
//! - [`queue`] — `Q`, modelled at the two properties I3 is about.
//! - [`collect`] — the cycle, and the refusal that stops it breaking its own
//!   invariant.
//! - [`invariants`] — I1, I2 and I3, as predicates over named state.
//! - [`publish`] — the half of RFC 0060's publish sequence `blob/` does not own.
//!
//! # Determinism
//!
//! Nothing under `zone/src/` names `f_env` at all, reads a clock, draws a
//! value, or iterates a `HashMap`: every collection here is a `BTreeMap` or a
//! `BTreeSet`, the sweep's selection order is stated (`live_bytes` ascending,
//! ties by ascending zone index) and the fill's is stated (ascending zone
//! index), because RFC 0004 says an order nobody stated is an order the map
//! chose. Two runs of one workload lay a device out identically, which is what
//! `E2-P06` compares. The tests draw their objects and their pin choices from a
//! seeded `Env`, which is the opposite obligation and is met in `zone/tests/`.
//!
//! # Where the numbers come from, and where they do not
//!
//! `zone/tests/cycle.rs` runs the phased workload RFC 0059 requires — *fill
//! with the collector held, collect to quiescence with the client idle, count*
//! — and prints the device bytes, the application bytes and their ratio.
//! **That ratio is measured against the modelled device in this crate and not
//! against QEMU's zoned virtio-blk**, so it is not claim 0016's number and this
//! crate does not register one: claim 0016 counts both sides with
//! `query-blockstats` on an emulated device inside a guest, and a number taken
//! at a different boundary is a different number however similar it looks. What
//! the host figure is for is the ratio's *shape* — how much of it is data, how
//! much copy-forward and how much block padding — which is the part that is a
//! property of this design rather than of a device.
//!
//! # What `E2-B02` names that is not here
//!
//! Said in one place so that a reader does not have to find it by looking:
//!
//! - **The five opcodes answered on the wire.** `user/virtio-blk` numbers
//!   `FLUSH`, `ZONE_APPEND`, `ZONE_FINISH`, `ZONE_RESET` and `ZONE_REPORT` and
//!   answers two of seven. [`device::Zoned`] is the shape a driver that
//!   answered them would present; nothing here widens `op::known`.
//! - **The QEMU boot.** The development image's QEMU is bookworm's 7.2 and
//!   zoned virtio-blk emulation is materially better from QEMU 8;
//!   `docker/README.md` names the base change. The boot the spec asks for is of
//!   `user/objects`, which is `E2-B08` and does not exist.
//! - **`ROOT_CARRY` and the root-zone wrap**, and the resolving half of
//!   verify-before-accept — [`publish`]'s last two paragraphs.
//! - **Claim 0016.** Registering it is a diff in `claims/`, and the number
//!   above is not the number that claim is about.

#![no_std]

extern crate alloc;

pub mod collect;
pub mod device;
pub mod invariants;
pub mod map;
pub mod mark;
pub mod publish;
pub mod queue;
pub mod roots;
