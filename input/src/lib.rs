// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One time source in the whole input path, and what is derived from it.
//!
//! An input event is stamped at the driver, at interrupt time, and nowhere
//! else. Every later stage — the ring, the compositor's queue, the prediction,
//! the late-latch — reads that stamp and takes no clock of its own. The reason
//! is not tidiness: a second reading anywhere in the path silently converts a
//! measured latency into the sum of a measurement and a scheduling delay, and
//! the sum is the number that would be published.
//!
//! So the rule is enforced by a lint rather than stated in a comment, and the
//! lint has a fixture that makes it fail, because a lint nobody has seen go red
//! is a lint nobody has tested.
//!
//! Under the simulator the stamp is `f_env::Env`'s virtual time, which is what
//! makes a prediction reproducible from a seed on both architectures. A
//! prediction whose error could not be replayed would be a prediction no test
//! could bound, and *bounded* is the whole claim this path makes.

#![no_std]

pub mod predict;
pub mod stamp;
