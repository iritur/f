// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Architecture support.
//!
//! AArch64 is a first-class target rather than a portability afterthought:
//! x86-64 total-store-order hides the entire class of memory-ordering bug the
//! ring protocol is exposed to, so a weak-memory target in CI is the only
//! configuration in which those tests mean anything.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;
