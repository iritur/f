// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Binding drivers to devices, by what a manifest declared and never by what a
//! scan reported first.
//!
//! # The rule, and the failure it exists to remove
//!
//! A PCI scan is a **discovery order**. It depends on which bridge answered
//! first, on how the firmware numbered the buses, and on which slot somebody
//! put a card in — none of which is a property of the generation root. If any
//! of it reached the topology, then `E2-B05`'s exit — *the same root produces a
//! byte-identical topology* — would be a property of the machine's wiring
//! rather than of the hash, and it would hold on one machine and quietly stop
//! holding on the next.
//!
//! So the order is stated rather than discovered, in two halves:
//!
//! - **Between components**, the topology's own canonical order, which is the
//!   record tree's — components sorted bytewise on the zero-padded name, which
//!   is what `f_generation::record` already enforces.
//! - **Within a component**, devices sorted by their **declared identity**:
//!   bus, device and function, as a [`BTreeMap`] key. Never the order the scan
//!   reported them in. [`Bus`] is that map, and it is the reason this crate
//!   takes `alloc` at all.
//!
//! A bus is therefore a *set* to this crate and never a sequence. [`Bus::saw`]
//! may be called in any order at all and the map is the same map — which is the
//! property `assembler/tests/assemble.rs` shuffles a bus under a seeded `Env`
//! to demonstrate, rather than asserting it about one hand-written order.
//!
//! # Two drivers claiming one device
//!
//! Refused, and the refusal that matters is `cargo xtask lint-manifests`'s: two
//! manifests declaring one `(vendor, device)` pair is a compile-time finding,
//! named in RFC 0065, because at boot the only things available to break the
//! tie are a scan's order and the topology's, and a topology broken by either
//! is not a function of the root.
//!
//! [`Refusal::Claimed`] below is the same rule seen from run time, and it is
//! not redundant: this crate can be handed a module built by something that is
//! not this tree's `xtask`, and a check that exists only in the build is a check
//! an attacker or a mistake routes around by not running the build. It reports
//! rather than picks, which is the whole difference.
//!
//! It is also **weaker than the build-time one, and the comment inside [`bind`]
//! says exactly where**: it catches two components declaring the same part,
//! where every match collides, and it can miss declarations that overlap
//! without being equal. That gap is not reachable from this tree, because
//! `lint-manifests` makes an overlapping declaration unbuildable — and saying
//! which of the two checks is the complete one is worth more than implying both
//! are.
//!
//! # A part in two slots
//!
//! A component binds the **lowest matching address** and no more, and a second
//! device of the same part is left unbound. That is a real limitation and it is
//! stated rather than discovered: two identical cards want two instances of the
//! driver, two instances want two members, and two members want two names —
//! which the record tree refuses, because names are unique. *Reversal:* a
//! topology that can name an instance separately from a component, which is a
//! node kind and therefore an RFC.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::topology::Assembly;

/// Where a device is, as a bus reports it.
///
/// Ordered, and the derived order is the one the binding uses: bus first, then
/// device, then function, which is how a person reads `00:04.0` and how
/// `kernel::arch::x86_64::pci::Bdf` packs a requester id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Address {
    /// Which bus.
    /// Unit: none — a PCI bus number, 0 to 255. Not a quantity.
    pub bus: u8,
    /// Which device on it.
    /// Unit: none — a PCI device number, 0 to 31.
    pub device: u8,
    /// Which function of that device.
    /// Unit: none — a PCI function number, 0 to 7.
    pub function: u8,
}

/// One device a scan found: where it is, and what it says it is.
///
/// The two halves are kept apart on purpose. The *where* is discovered and may
/// differ between two machines running one generation; the *what* is what a
/// manifest declares and matches on. A type that merged them would be a type in
/// which a manifest could accidentally name a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Discovered {
    /// Where it is. Discovered, never declared.
    /// Unit: none — a bus address.
    pub at: Address,
    /// Who made it.
    /// Unit: none — a PCI vendor identifier as the bus reports it.
    pub vendor: u16,
    /// Which part.
    /// Unit: none — a PCI device identifier as the bus reports it.
    pub device: u16,
}

/// What a scan found, as a set keyed by address.
///
/// The `BTreeMap` is the mechanism and not the container: RFC 0004 forbids a
/// `HashMap` because iteration order is seeded per process, and what is wanted
/// here is stronger than *not seeded* — it is *the address order, whatever
/// order the scan ran in*. A `Vec` in scan order with a sort at the end would
/// reach the same place and would leave a `Vec` in scan order lying around for
/// somebody to iterate.
#[derive(Clone, Debug, Default)]
pub struct Bus {
    by_address: BTreeMap<Address, Discovered>,
}

impl Bus {
    /// An empty bus.
    #[must_use]
    pub fn new() -> Self {
        Self { by_address: BTreeMap::new() }
    }

    /// Record one device a scan found.
    ///
    /// Returns what was already at that address, if anything. A caller that
    /// gets `Some` has scanned one address twice and been told two different
    /// things about it, which is a bug in the scan rather than in the topology
    /// — reported here rather than resolved, because this crate has no basis
    /// for preferring either answer.
    pub fn saw(&mut self, device: Discovered) -> Option<Discovered> {
        self.by_address.insert(device.at, device)
    }

    /// Every device, in address order. Never in scan order: there is no scan
    /// order in this type to return.
    pub fn devices(&self) -> impl Iterator<Item = &Discovered> {
        self.by_address.values()
    }

    /// How many devices were found.
    /// Unit: devices.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_address.len()
    }

    /// Is the bus empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_address.is_empty()
    }
}

/// Why a binding could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Two components' declarations match one device. Carries the address and
    /// the two member indices, lower first.
    ///
    /// `cargo xtask lint-manifests` refuses this at compile time and RFC 0065
    /// says why that is the refusal that matters. This one exists because a
    /// build-time check is a check that is skipped by not running the build.
    Claimed(Address, u16, u16),
}

impl Refusal {
    /// A line for whoever is holding the machine that would not bind.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Claimed(_, _, _) => {
                "two components declare the same device, and nothing here is entitled to choose"
            }
        }
    }
}

/// Bind every component that declares a device to the device it declared.
///
/// Iterates the topology's members in **canonical order** and, within each,
/// the bus in **address order**. Neither loop can see a discovery order,
/// because neither structure holds one.
///
/// A component that declares no device is untouched. A component that declares
/// one the bus does not have is left [`Instance::bound`](crate::Instance::bound)
/// `None`, which is not a refusal here: a driver whose part is absent is a
/// driver that cannot start, and [`crate::start`] is where that becomes a
/// state.
///
/// # Errors
///
/// [`Refusal::Claimed`] when two components match one device.
pub fn bind(assembly: &mut Assembly, bus: &Bus) -> Result<usize, Refusal> {
    // Which member has taken which address. A map rather than a flag on the
    // device, because the refusal has to be able to name *both* claimants and a
    // flag can only name that there was one.
    let mut taken: BTreeMap<Address, u16> = BTreeMap::new();
    let mut bound = 0;

    let indices: Vec<u16> = assembly.members().iter().map(|member| member.index).collect();
    for index in indices {
        let Some(member) = assembly.member(index) else { continue };
        if member.record.bindings().is_empty() {
            continue;
        }
        let declared: Vec<f_abi::manifest::Binding> = member.record.bindings().to_vec();

        let mut first: Option<Address> = None;
        for device in bus.devices() {
            if !declared.iter().any(|want| want.matches(device.vendor, device.device)) {
                continue;
            }
            if let Some(other) = taken.get(&device.at) {
                let (low, high) = if *other < index { (*other, index) } else { (index, *other) };
                return Err(Refusal::Claimed(device.at, low, high));
            }
            // The lowest matching address, and the loop keeps going rather than
            // stopping here, so that a component which also matches a *later*
            // address still sees that address taken.
            //
            // **What that does not buy, said plainly.** Only the address a
            // component actually binds goes into `taken`, so two components
            // whose declarations overlap without being equal — A matches
            // {X, Y}, B matches {Y} alone — are caught when A is asked second
            // and not when A is asked first. The order is canonical, so the
            // answer is the same on every machine from one root and byte
            // identity is unaffected; what is not true is that this check is
            // complete. The complete one is `cargo xtask lint-manifests`, which
            // refuses two manifests declaring one part and therefore makes
            // overlapping-but-unequal declarations unbuildable in this tree.
            // This check is for a module this tree's build did not produce, and
            // it catches the case that matters there: two components declaring
            // the same part, where every match collides.
            if first.is_none() {
                first = Some(device.at);
            }
        }

        if let Some(at) = first {
            taken.insert(at, index);
            if let Some(member) = assembly.member_mut(index) {
                member.bound = Some(at);
            }
            bound += 1;
        }
    }
    Ok(bound)
}

/// Does this member declare a device it was not bound to?
///
/// The one question [`crate::start`] asks of a binding, kept here so that
/// *declared a device and has none* is defined once beside the binding rather
/// than twice beside its consumers.
#[must_use]
pub fn wanted_and_absent(assembly: &Assembly, index: u16) -> bool {
    assembly
        .member(index)
        .is_some_and(|member| !member.record.bindings().is_empty() && member.bound.is_none())
}
