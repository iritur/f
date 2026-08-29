// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The first user component.
//!
//! It exists at M0 as a placeholder that proves one structural property: a
//! component above the frame can speak the ring protocol without a single line
//! of `unsafe`. That is enforced rather than asserted — this crate inherits
//! `unsafe_code = "forbid"`, and `cargo xtask lint-unsafe` fails the build if
//! any crate outside the frame acquires it.
//!
//! At M3 this becomes a real process loaded from a boot module. At M4 it is the
//! first thing to be handed a capability table and asked to prove it cannot
//! exceed it.

#![no_std]

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
