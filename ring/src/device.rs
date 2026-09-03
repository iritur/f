// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Granted memory a component may touch without writing `unsafe`: a device's
//! register window, and a region a device transfers through.
//!
//! # Why this exists at all
//!
//! A driver above the frame inherits `unsafe_code = "forbid"` and the property
//! is enforced rather than asserted — `cargo xtask lint-unsafe` is the backstop
//! and the workspace lint is the wall. So a component cannot dereference a
//! register, cannot write a descriptor through a raw pointer, and cannot read a
//! status byte a device wrote. Before this module the answer to *how does a
//! driver touch its device, then* was that there was no answer, and E1-B05
//! recorded the same wall one floor up: a supervisor cannot adopt its own
//! control ring, so the restart policy stayed in the frame.
//!
//! This is the answer for the device half, and it is not a new argument. The
//! tree already makes it twice:
//!
//! - [`f_abi::door::call`] is one instruction on the frame's side of the
//!   boundary, because *the calling convention is the platform's, not the
//!   application's* — and a component that hand-rolled it could get it wrong in
//!   a way only the kernel could detect.
//! - [`f_abi::state::Reader`] is a **safe** function over an address the frame
//!   mapped, and its own comment says why: the obligation is discharged against
//!   a contract the frame keeps, and a component that invents an address gets a
//!   page fault, which is the defined machine outcome the whole isolation suite
//!   rests on.
//!
//! [`Window`] and [`Region`] are the third and fourth. Written once here,
//! reviewed as part of the frame, and used by every driver — against the
//! alternative, which is every driver containing the same `unsafe` block
//! written slightly differently. RFC 0033 is the argument in full, including
//! what it costs and what would reverse it.
//!
//! # What is checked and what is contract
//!
//! **Checked, on every access:** that the offset and the width fall inside the
//! length the constructor was given, and that the offset is aligned to the
//! width. Both are refusals rather than panics — `ARGUMENT/BAD_ADDRESS` — for
//! the reason the rest of this crate refuses rather than panics: a driver is a
//! component, a component that panics ends, and an arithmetic slip in a driver
//! should be a completion its client can read.
//!
//! **Contract, once, at the constructor:** that `base` names `len` bytes the
//! frame mapped for this component in answer to a capability it holds. That is
//! the [`Reader`](f_abi::state::Reader) sentence unchanged. It is not *sound*
//! by Rust's rules and saying so is the point: what makes it acceptable is that
//! a component which invents an address takes a page fault at ring 3, and the
//! frame kills it — `cargo xtask user` is seven boots of exactly that outcome.
//!
//! # Why a window and a region are two types
//!
//! Because they differ in who else is writing, and the difference decides
//! whether an access may be reordered or elided.
//!
//! A [`Window`] is **registers**. Every access is volatile because a read can
//! have a side effect, two reads of one address can answer differently, and the
//! device is watching the order. Nothing is cached and nothing is merged.
//!
//! A [`Region`] is **memory a device also reads and writes**. Accesses are
//! volatile for the same reason, and it carries a second address: where the
//! *device* sees the same bytes. That is [`Domains::map`](crate::registry::Domains::map)'s
//! answer, and carrying it here is what stops a driver assuming the device's
//! address space is its own — the assumption this build happens to satisfy and
//! `kernel/src/iommu.rs` writes a reversal condition for.
//!
//! Neither type hands out a slice, and that is deliberate: a slice asserts
//! exclusive access to memory whose whole purpose is that something else writes
//! it, which is the same reason [`Mapping`](crate::Mapping) hands out atomics
//! and [`UnsafeCell`](core::cell::UnsafeCell) rather than references.
//!
//! # What neither of them is
//!
//! A way to reach a *client's* buffer. A driver on the registered path has no
//! mapping of its clients' memory at all — [`Reach`](crate::registry::Reach) is
//! an address and deliberately not a slice, so that a copy has no expression —
//! and nothing in this module changes that. A [`Region`] is memory the
//! component was granted: its own queue, its own request headers. The bytes of
//! a read or a write never appear in one.

use core::sync::atomic::{Ordering, fence};

use f_abi::error;

/// Refuse an access this type will not perform.
///
/// One function so that the two types cannot drift about what "inside" means,
/// and so the reason is written once: an offset rounded down to an alignment,
/// or a length clipped to fit, is the helpful behaviour that turns a driver's
/// arithmetic slip into a register write somewhere else. R04.
const fn bounded(offset: u32, width: u32, len: u32) -> Result<(), i32> {
    let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
    if !offset.is_multiple_of(width) {
        return Err(bad);
    }
    match offset.checked_add(width) {
        Some(end) if end <= len => Ok(()),
        _ => Err(bad),
    }
}

/// Refuse a base address no accessor can be stated against.
///
/// `align` is the strongest alignment any access through the returned value
/// will need, checked once here rather than at every access — an access is
/// aligned relative to the base, so a base that is not itself aligned makes
/// every per-access check a lie.
const fn addressable(base: u64, len: u32, align: u64) -> Result<(), i32> {
    let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
    if base == 0 || len == 0 || !base.is_multiple_of(align) {
        return Err(bad);
    }
    Ok(())
}

/// A device's memory-mapped registers, as a component may touch them.
///
/// Copy, and deliberately: it is three words describing a window the frame
/// mapped, it owns nothing, and a driver that has to thread one `&mut` through
/// its whole initialisation would end up with a borrow graph rather than a
/// device. Nothing here can be used to *acquire* a window — [`Window::at`]'s
/// contract is the only way in, and its caller is the frame.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    base: u64,
    len: u32,
}

impl Window {
    /// Bind to a register window the frame mapped.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for an address no window can be stated against:
    /// zero, a zero length, or a base that is not four-byte aligned — which is
    /// the widest single access this type performs, because a sixty-four-bit
    /// device register is written as two words and
    /// [`Window::write64`] says why.
    ///
    /// # Why this is safe to call
    ///
    /// The module documentation, and [`f_abi::state::Reader`]'s own paragraph,
    /// which this repeats rather than gestures at: `base` is an address the
    /// frame mapped in answer to a capability the caller held, and a component
    /// that invents one takes a page fault.
    pub const fn at(base: u64, len: u32) -> Result<Self, i32> {
        match addressable(base, len, 4) {
            Ok(()) => Ok(Self { base, len }),
            Err(refused) => Err(refused),
        }
    }

    /// Bytes in the window. Unit: bytes.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.len
    }

    /// Never — [`Window::at`] refuses a zero length — but the pair is
    /// conventional and the lint asks for it.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// A window over part of this one.
    ///
    /// What a driver does when a device describes several structures inside one
    /// base-address register: the frame maps the pages, and the driver narrows.
    /// Narrowing only — the result is always inside this window — so a driver
    /// cannot widen its way out of what it was granted, which is the property
    /// that makes handing a sub-window to another part of the driver safe to do
    /// at all.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for a range that leaves this window, or an offset
    /// that would make the result unaligned.
    pub const fn slice(&self, offset: u32, len: u32) -> Result<Self, i32> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        if len == 0 {
            return Err(bad);
        }
        match offset.checked_add(len) {
            Some(end) if end <= self.len => {}
            _ => return Err(bad),
        }
        Self::at(self.base.wrapping_add(offset as u64), len)
    }

    /// Read one byte.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for an offset outside the window.
    pub fn read8(&self, offset: u32) -> Result<u8, i32> {
        bounded(offset, 1, self.len)?;
        // SAFETY: the type's contract — `base` names `len` bytes the frame
        // mapped for this component — and `bounded` has just established that
        // this byte is inside `len`. Volatile because the reader is a device
        // register: a read may have a side effect and two reads may differ.
        Ok(unsafe { (self.base.wrapping_add(offset as u64) as *const u8).read_volatile() })
    }

    /// Read two bytes.
    ///
    /// # Errors
    ///
    /// As [`Window::read8`], and for an offset that is not two-byte aligned.
    pub fn read16(&self, offset: u32) -> Result<u16, i32> {
        bounded(offset, 2, self.len)?;
        // SAFETY: as `read8`, at two bytes, and `bounded` checked the
        // alignment this load needs as well as the extent.
        Ok(unsafe { (self.base.wrapping_add(offset as u64) as *const u16).read_volatile() })
    }

    /// Read four bytes.
    ///
    /// # Errors
    ///
    /// As [`Window::read16`], at four bytes.
    pub fn read32(&self, offset: u32) -> Result<u32, i32> {
        bounded(offset, 4, self.len)?;
        // SAFETY: as `read16`, at four bytes.
        Ok(unsafe { (self.base.wrapping_add(offset as u64) as *const u32).read_volatile() })
    }

    /// Write one byte.
    ///
    /// Takes `&self` rather than `&mut self`, which is the honest signature: a
    /// device register is not memory this component exclusively owns, and a
    /// `&mut` would be a claim of exclusivity that is false the moment the
    /// device is running. The same reason [`Mapping`](crate::Mapping) hands out
    /// atomics.
    ///
    /// # Errors
    ///
    /// As [`Window::read8`].
    pub fn write8(&self, offset: u32, value: u8) -> Result<(), i32> {
        bounded(offset, 1, self.len)?;
        // SAFETY: the type's contract and the bound just checked. Volatile
        // because the reader is a device: this store may not be elided,
        // reordered with its neighbours or merged with them.
        unsafe { (self.base.wrapping_add(offset as u64) as *mut u8).write_volatile(value) };
        Ok(())
    }

    /// Write two bytes.
    ///
    /// # Errors
    ///
    /// As [`Window::read16`].
    pub fn write16(&self, offset: u32, value: u16) -> Result<(), i32> {
        bounded(offset, 2, self.len)?;
        // SAFETY: as `write8`, at two bytes, aligned by `bounded`.
        unsafe { (self.base.wrapping_add(offset as u64) as *mut u16).write_volatile(value) };
        Ok(())
    }

    /// Write four bytes.
    ///
    /// # Errors
    ///
    /// As [`Window::read32`].
    pub fn write32(&self, offset: u32, value: u32) -> Result<(), i32> {
        bounded(offset, 4, self.len)?;
        // SAFETY: as `write16`, at four bytes.
        unsafe { (self.base.wrapping_add(offset as u64) as *mut u32).write_volatile(value) };
        Ok(())
    }

    /// Write eight bytes as two four-byte halves, low first.
    ///
    /// Not one eight-byte store, and the reason is the same one
    /// `kernel/src/arch/x86_64/dma.rs` records: a specification that defines a
    /// sixty-four-bit register usually permits it to be written as two words
    /// and fixes this order, and most device windows are implemented in
    /// thirty-two-bit registers. A single wide store is correct on the machines
    /// where it happens to work.
    ///
    /// # Errors
    ///
    /// As [`Window::read32`], for either half.
    pub fn write64(&self, offset: u32, value: u64) -> Result<(), i32> {
        // Both halves bounded before either is written, so a register whose
        // upper half falls outside the window is refused rather than
        // half-written — a device left holding half an address is worse than a
        // device left holding none.
        bounded(offset, 4, self.len)?;
        let upper = offset
            .checked_add(4)
            .ok_or(error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS))?;
        bounded(upper, 4, self.len)?;
        self.write32(offset, value as u32)?;
        self.write32(upper, (value >> 32) as u32)
    }
}

/// Memory a device transfers through, as a component may touch it.
///
/// Two addresses for one set of bytes: [`Region::device_at`] is where the
/// device sees them and everything else is where this component does. They are
/// equal in this build and the type does not assume it —
/// `kernel/src/iommu.rs` states the reversal, which is a device that cannot
/// address the whole of physical memory, and a driver written against this type
/// needs no change on the day that happens.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    base: u64,
    device: u64,
    len: u32,
}

impl Region {
    /// Bind to a region the frame granted and gave a device translation for.
    ///
    /// `device` is [`Domains::map`](crate::registry::Domains::map)'s answer for
    /// this region. Passing one the frame did not answer with is a driver
    /// pointing a device at memory of somebody else's choosing, and it is the
    /// attempt this whole task's second half exists to make fault rather than
    /// land: the device's domain is what refuses it, not this constructor,
    /// because a constructor that could tell would be a constructor with a page
    /// walk in it.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for either address being zero, a zero length, or
    /// a base or device address that is not sixteen-byte aligned — which is a
    /// virtqueue descriptor's alignment and the strongest thing placed in one
    /// of these.
    ///
    /// # Why this is safe to call
    ///
    /// As [`Window::at`].
    pub const fn at(base: u64, device: u64, len: u32) -> Result<Self, i32> {
        match addressable(base, len, 16) {
            Ok(()) => {}
            Err(refused) => return Err(refused),
        }
        match addressable(device, len, 16) {
            Ok(()) => Ok(Self { base, device, len }),
            Err(refused) => Err(refused),
        }
    }

    /// Bytes in the region. Unit: bytes.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.len
    }

    /// Never — [`Region::at`] refuses a zero length.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Where the device addresses `offset` bytes into this region.
    ///
    /// Unit: bytes, in the device's address space.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for an offset outside the region. Refused rather
    /// than answered, because the answer would be an address a driver would put
    /// in a descriptor: a device address computed past the end of a grant is
    /// exactly the descriptor this task's second half requires to fault, and
    /// producing one here would be the driver's own arithmetic doing what the
    /// hardware is there to stop.
    pub const fn device_at(&self, offset: u32) -> Result<u64, i32> {
        match bounded(offset, 1, self.len) {
            Ok(()) => Ok(self.device.wrapping_add(offset as u64)),
            Err(refused) => Err(refused),
        }
    }

    /// A region over part of this one, on both sides.
    ///
    /// What a driver does with the one untyped region its manifest routes it:
    /// `user/virtio-blk/manifest.toml` says *untyped rather than frames because
    /// the driver decides the split*, and this is the split. The component's
    /// address and the device's move together, which is the property that makes
    /// narrowing safe to hand to another part of a driver — a sub-region cannot
    /// name a byte the whole one did not.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for a range that leaves this region, or an offset
    /// that would leave either address unaligned.
    pub const fn slice(&self, offset: u32, len: u32) -> Result<Self, i32> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        if len == 0 {
            return Err(bad);
        }
        match offset.checked_add(len) {
            Some(end) if end <= self.len => {}
            _ => return Err(bad),
        }
        Self::at(
            self.base.wrapping_add(offset as u64),
            self.device.wrapping_add(offset as u64),
            len,
        )
    }

    /// Read one byte the device may have written.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for an offset outside the region.
    pub fn get8(&self, offset: u32) -> Result<u8, i32> {
        bounded(offset, 1, self.len)?;
        // SAFETY: the type's contract — `base` names `len` bytes the frame
        // granted this component — and `bounded` has established this byte is
        // inside them. Volatile because a device writes here: this load may not
        // be hoisted above the acquire fence a driver takes after a completion.
        Ok(unsafe { (self.base.wrapping_add(offset as u64) as *const u8).read_volatile() })
    }

    /// Read two bytes the device may have written.
    ///
    /// # Errors
    ///
    /// As [`Region::get8`], and for an offset that is not two-byte aligned.
    pub fn get16(&self, offset: u32) -> Result<u16, i32> {
        bounded(offset, 2, self.len)?;
        // SAFETY: as `get8`, at two bytes, aligned by `bounded`.
        Ok(unsafe { (self.base.wrapping_add(offset as u64) as *const u16).read_volatile() })
    }

    /// Read four bytes the device may have written.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`], at four bytes.
    pub fn get32(&self, offset: u32) -> Result<u32, i32> {
        bounded(offset, 4, self.len)?;
        // SAFETY: as `get16`, at four bytes.
        Ok(unsafe { (self.base.wrapping_add(offset as u64) as *const u32).read_volatile() })
    }

    /// Write one byte the device will read.
    ///
    /// `&self` for the reason [`Window::write8`] gives.
    ///
    /// # Errors
    ///
    /// As [`Region::get8`].
    pub fn put8(&self, offset: u32, value: u8) -> Result<(), i32> {
        bounded(offset, 1, self.len)?;
        // SAFETY: the type's contract and the bound just checked. Volatile
        // because a device reads here and this store may not be sunk past the
        // release fence that publishes it.
        unsafe { (self.base.wrapping_add(offset as u64) as *mut u8).write_volatile(value) };
        Ok(())
    }

    /// Write two bytes the device will read.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`].
    pub fn put16(&self, offset: u32, value: u16) -> Result<(), i32> {
        bounded(offset, 2, self.len)?;
        // SAFETY: as `put8`, at two bytes, aligned by `bounded`.
        unsafe { (self.base.wrapping_add(offset as u64) as *mut u16).write_volatile(value) };
        Ok(())
    }

    /// Write four bytes the device will read.
    ///
    /// # Errors
    ///
    /// As [`Region::get32`].
    pub fn put32(&self, offset: u32, value: u32) -> Result<(), i32> {
        bounded(offset, 4, self.len)?;
        // SAFETY: as `put16`, at four bytes.
        unsafe { (self.base.wrapping_add(offset as u64) as *mut u32).write_volatile(value) };
        Ok(())
    }

    /// Write eight bytes the device will read.
    ///
    /// One store here and two in [`Window::write64`], and the difference is the
    /// reader: this is ordinary memory that a device reads through the same
    /// coherent path this core writes it on, where a register window is a
    /// device's own decode logic that may only be thirty-two bits wide.
    ///
    /// # Errors
    ///
    /// As [`Region::get32`], at eight bytes.
    pub fn put64(&self, offset: u32, value: u64) -> Result<(), i32> {
        bounded(offset, 8, self.len)?;
        // SAFETY: as `put32`, at eight bytes, aligned by `bounded`.
        unsafe { (self.base.wrapping_add(offset as u64) as *mut u64).write_volatile(value) };
        Ok(())
    }

    /// Publish everything written so far, then store `value` at `offset`.
    ///
    /// The same discipline the ring itself rests on, stated once so that a
    /// driver does not have to remember it at each virtqueue: the entry is
    /// written, then the cursor that makes it visible is written after a
    /// `Release` fence. A device has a weaker relationship to this core's store
    /// buffer than another core does, so the fence is what makes the bytes it
    /// reads the bytes that were written.
    ///
    /// A fence rather than a release *store*, because what is being ordered is
    /// a group of plain volatile writes rather than one atomic — the same shape
    /// `dma.rs` uses and the same reason.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`].
    pub fn publish16(&self, offset: u32, value: u16) -> Result<(), i32> {
        bounded(offset, 2, self.len)?;
        fence(Ordering::Release);
        self.put16(offset, value)
    }

    /// Read a cursor a device published, and everything it published before it.
    ///
    /// The `Acquire` half of [`Region::publish16`]. A driver that read a used
    /// index and then read the bytes the device wrote, with nothing between
    /// them, would be relying on the compiler and the processor to not do the
    /// thing they are both permitted to do.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`].
    pub fn consume16(&self, offset: u32) -> Result<u16, i32> {
        let value = self.get16(offset)?;
        fence(Ordering::Acquire);
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sixty-four bytes this test owns, at an alignment both constructors
    /// accept.
    ///
    /// The alignment is part of the fixture rather than incidental: a `[u8;
    /// 64]` on the stack is aligned to one byte as far as the language is
    /// concerned, so a test that used one would pass or fail on where the
    /// compiler happened to put it — which is the shape of flake this tree
    /// spends most of its apparatus avoiding.
    #[repr(align(16))]
    struct Owned([u8; 64]);

    impl Owned {
        const fn new() -> Self {
            Self([0; 64])
        }

        fn address(&mut self) -> u64 {
            self.0.as_mut_ptr() as usize as u64
        }
    }

    /// A window over this test's own stack, which is the only address a host
    /// test can honestly claim the contract for: it is memory this program
    /// owns, for as long as the value lives, and nothing else writes it.
    ///
    /// That is weaker than what a driver gets and it is the right weakness:
    /// what these tests establish is the *arithmetic* — which offsets are
    /// refused and which are not — and no host test can establish anything
    /// about a device.
    fn over(bytes: &mut Owned) -> Window {
        Window::at(bytes.address(), 64).expect("an aligned, non-empty window")
    }

    #[test]
    fn an_address_no_accessor_can_be_stated_against_is_refused() {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        assert_eq!(Window::at(0, 64).map(|_| ()), Err(bad), "the null page");
        assert_eq!(Window::at(0x1000, 0).map(|_| ()), Err(bad), "no bytes");
        assert_eq!(Window::at(0x1002, 64).map(|_| ()), Err(bad), "not four-byte aligned");
        assert!(Window::at(0x1000, 64).is_ok());

        assert_eq!(Region::at(0x1000, 0, 64).map(|_| ()), Err(bad), "no device address");
        assert_eq!(Region::at(0x1004, 0x1004, 64).map(|_| ()), Err(bad), "not descriptor-aligned");
        assert!(Region::at(0x1000, 0x1000, 64).is_ok());
    }

    #[test]
    fn an_access_past_the_grant_is_refused_and_not_clipped() {
        // The refusal the whole type exists for. A length clipped to fit is a
        // driver told its transfer succeeded over fewer bytes than it named,
        // and an offset rounded down is a register write somewhere else.
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let mut bytes = Owned::new();
        let window = over(&mut bytes);

        assert_eq!(window.read8(64), Err(bad), "one past the end");
        assert_eq!(window.read32(61), Err(bad), "four bytes starting three from the end");
        assert_eq!(window.write32(u32::MAX, 0), Err(bad), "an offset that would overflow");
        assert!(window.read32(60).is_ok(), "the last word is inside");
    }

    #[test]
    fn an_unaligned_access_is_refused_rather_than_performed() {
        // Not a portability nicety. An unaligned volatile load through a raw
        // pointer is undefined behaviour, so this check is what stands between
        // a driver's arithmetic slip and a soundness hole in the frame.
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let mut bytes = Owned::new();
        let window = over(&mut bytes);

        assert_eq!(window.read16(1), Err(bad));
        assert_eq!(window.read32(2), Err(bad));
        assert_eq!(window.write16(3, 0), Err(bad));
        assert!(window.read16(2).is_ok());
        assert!(window.read32(4).is_ok());
    }

    #[test]
    fn a_sixty_four_bit_register_is_refused_whole_rather_than_half_written() {
        // A device left holding half an address is worse than one holding
        // none: it has a queue address whose upper word is whatever the reset
        // value was, which is a translation somebody else's memory may be at.
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let mut bytes = Owned::new();
        let window = over(&mut bytes);

        assert_eq!(
            window.write64(60, 0xAAAA_BBBB_CCCC_DDDD),
            Err(bad),
            "the upper half is outside"
        );
        assert_eq!(window.read32(60), Ok(0), "and the lower half was not written either");

        window.write64(56, 0xAAAA_BBBB_CCCC_DDDD).expect("both halves inside");
        assert_eq!(window.read32(56), Ok(0xCCCC_DDDD), "low half first");
        assert_eq!(window.read32(60), Ok(0xAAAA_BBBB), "high half second");
    }

    #[test]
    fn a_sub_window_narrows_and_cannot_widen() {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let mut bytes = Owned::new();
        let window = over(&mut bytes);

        let inner = window.slice(16, 16).expect("inside");
        assert_eq!(inner.len(), 16);
        assert_eq!(inner.read8(16), Err(bad), "the sub-window ends where it was told to");

        assert_eq!(window.slice(48, 32).map(|_| ()), Err(bad), "a range that leaves the window");
        assert_eq!(window.slice(0, 0).map(|_| ()), Err(bad), "a window of nothing");

        // What a narrowed window sees is what the outer one wrote at the same
        // absolute place — the check that the arithmetic is a narrowing rather
        // than a new base somebody computed.
        window.write32(16, 0x1234_5678).expect("inside");
        assert_eq!(inner.read32(0), Ok(0x1234_5678));
    }

    #[test]
    fn a_device_address_is_answered_for_the_region_and_refused_past_it() {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let mut bytes = Owned::new();
        let region = Region::at(bytes.address(), 0x8000_0000, 64).expect("an aligned region");

        assert_eq!(region.device_at(0), Ok(0x8000_0000));
        assert_eq!(region.device_at(63), Ok(0x8000_003F));
        assert_eq!(region.device_at(64), Err(bad), "one past the grant");
        assert_eq!(region.device_at(u32::MAX), Err(bad));

        // The two addresses are separate: writing through the component's side
        // does not move the device's, which is the whole reason the type
        // carries both rather than assuming they are equal.
        region.put32(0, 0xDEAD_BEEF).expect("inside");
        assert_eq!(region.get32(0), Ok(0xDEAD_BEEF));
        assert_eq!(region.device_at(0), Ok(0x8000_0000));
    }

    #[test]
    fn a_sub_region_narrows_both_addresses_together() {
        // The property a driver's split rests on: the device's view and the
        // component's move by the same amount, so a sub-region names the same
        // bytes on both sides. A slice that moved only one would be a
        // descriptor pointing at the right length of the wrong memory.
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let mut bytes = Owned::new();
        let region = Region::at(bytes.address(), 0x2_0000, 64).expect("an aligned region");

        let inner = region.slice(32, 32).expect("inside");
        assert_eq!(inner.len(), 32);
        assert_eq!(inner.device_at(0), Ok(0x2_0020));
        assert_eq!(inner.device_at(32), Err(bad), "and it ends where it was told to");

        region.put32(32, 0x0BAD_F00D).expect("inside");
        assert_eq!(inner.get32(0), Ok(0x0BAD_F00D));

        assert_eq!(region.slice(48, 32).map(|_| ()), Err(bad), "a range that leaves the region");
        assert_eq!(region.slice(8, 32).map(|_| ()), Err(bad), "an offset that unaligns it");
    }

    #[test]
    fn a_published_cursor_is_read_back_by_the_matching_consume() {
        // The pair, over one region, which is what a virtqueue does with an
        // available index. What this checks is that the value written is the
        // value read back. What it does **not** check is the ordering, and the
        // honest statement is that nothing in this tree does: this test is one
        // thread with no device on the other side, so it passes byte for byte
        // with both fences replaced by `Relaxed`; the AArch64 job runs this same
        // host test and therefore cannot tell them apart either; and the litmus
        // job's pairs are the ring's, between two cores, not a driver's between
        // a core and a device. The fences are here because `ring`'s own module
        // documentation requires them, and they are unverified.
        //
        // What would verify them is a mutation rather than another assertion —
        // `mutate-relaxed-device-publish` beside `mutate-relaxed-submission` in
        // `MUTATIONS`, weakening `Region::publish16`, run under
        // `cargo xtask blk`, which is the one harness in this tree with a real
        // device at the other end of the pair. That needs `mutate` to learn to
        // boot a machine carrying a disk, which it cannot today; it is written
        // here rather than claimed as covered because `docs/test-taxonomy.md`
        // already has the row this gap belongs to.
        let mut bytes = Owned::new();
        let region = Region::at(bytes.address(), 0x1_0000, 64).expect("an aligned region");

        region.put64(0, 0x1122_3344_5566_7788).expect("inside");
        region.publish16(32, 7).expect("inside");
        assert_eq!(region.consume16(32), Ok(7));
        assert_eq!(region.get32(0), Ok(0x5566_7788));
    }
}
