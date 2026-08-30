// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The first user component.
//!
//! It exists at M0 as a placeholder that proves one structural property: a
//! component above the frame can speak the ring protocol without a single line
//! of `unsafe`. That is enforced rather than asserted — this crate inherits
//! `unsafe_code = "forbid"`, and `cargo xtask lint-unsafe` fails the build if
//! any crate outside the frame acquires it.
//!
//! At E0-B10 that stopped being the whole of it. `component` is this crate as
//! something that *runs*: a flat image, linked by `user/init/link.ld`, handed to
//! the machine by the boot loader as a file and copied into a frame the process
//! was granted. The kernel does not contain it. What is left here is the part
//! that can be tested on the host, which is the protocol arithmetic — and that
//! split is worth keeping, because it is the only part a host test can reach.

#![no_std]

// The component is x86-64's, and only because the door is. Nothing in
// `component.rs` is architecture-specific — it is capability calls and a polling
// loop — but the one instruction underneath it is, and `f_abi::door::call` is
// compiled only where there is a frame to call. On any other target this crate
// is the protocol arithmetic and its tests, which is what the AArch64 job runs
// it for.
//
// *Reversal:* an AArch64 frame. At that point `door::call` grows a second
// implementation and this gate goes away — inventing one before the frame
// exists would be an ABI with nothing on the other side of it.
#[cfg(target_arch = "x86_64")]
pub mod component;

use f_abi::{Sqe, class, flags};

/// Build a no-op submission carrying a deadline.
///
/// Every request in the system carries a class and a deadline, including the
/// ones that do nothing. The field is not an optimisation to be added later:
/// deadline propagation across rings is what stops a hard-class task from
/// inverting priority behind batch work in some other service queue.
#[must_use]
pub fn nop(user_data: u64, deadline: u64) -> Sqe {
    Sqe { opcode: 0, flags: flags::NO_CQE, class: class::SOFT, user_data, deadline, ..Sqe::ZERO }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_component_speaks_the_protocol_without_unsafe() {
        let sqe = nop(7, 1_000_000);
        assert_eq!(sqe.user_data, 7);
        assert_eq!(sqe.deadline, 1_000_000);
        assert_eq!(sqe.class, class::SOFT);
        assert_eq!(sqe.flags & flags::NO_CQE, flags::NO_CQE);
    }
}
