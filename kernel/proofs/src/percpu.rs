// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The one name `kernel/src/cap.rs` takes from `kernel/src/percpu.rs`.
//!
//! `cap.rs` declares `static TABLE: PerCpu<Table>` so that the table of the
//! process running on a core is reachable without an allocator. Nothing a
//! proof does goes near it: a harness builds its own [`crate::cap::Table`] and
//! drives it directly, because the five properties are about what a table does
//! with a handle and not about which core is asking.
//!
//! So this holds nothing and answers with a null pointer. That is a stand-in
//! that cannot be mistaken for the real one — a shard that quietly returned a
//! usable pointer to a single shared table would be a fixture pretending to be
//! per-CPU, which is worse than one that obviously is not.

use core::marker::PhantomData;
use core::mem::ManuallyDrop;

/// A per-core slot, minus the cores.
///
/// `PhantomData<fn() -> T>` rather than `T`, so this is `Sync` for every `T`
/// without an `unsafe impl` — which is the whole reason it stores nothing.
pub struct PerCpu<T> {
    _held: PhantomData<fn() -> T>,
}

impl<T> PerCpu<T> {
    /// A slot holding the value every core starts at, which this drops.
    ///
    /// `ManuallyDrop` because a `const fn` may not run a destructor for a
    /// generic parameter, and the alternative — a bound saying `T` has none —
    /// would be this file dictating something to `cap.rs`.
    pub const fn new(value: T) -> Self {
        let _ = ManuallyDrop::new(value);
        Self { _held: PhantomData }
    }

    /// Null. See the module comment.
    pub fn mine(&self) -> *mut T {
        core::ptr::null_mut()
    }

    /// Null. See the module comment.
    pub fn at(&self, _cpu: usize) -> *mut T {
        core::ptr::null_mut()
    }
}
