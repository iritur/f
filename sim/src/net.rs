// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A network interface, as its driver sees it: a header, a frame, and a used
//! entry that says nothing.
//!
//! # The one thing this model is for
//!
//! A block device tells its driver what happened, in a status byte. **A network
//! interface does not.** virtio-net's transmit queue publishes a used entry with
//! a length of zero and no status anywhere: a frame the link dropped, a frame
//! the switch discarded and a frame delivered intact are the same completion,
//! and a driver that reported otherwise would be inventing information.
//!
//! That is the protocol, not a hole in the model, and it is why this device is
//! worth having beside the block one. `docs/design/proving-ground.html` lists
//! *drop, duplicate, reorder, corrupt, partition, delayed delivery* as the
//! faults to inject into a network interface, and every one of them is invisible
//! from the queue. A client that expects to hear about them is a client with a
//! bug, and only a model that stays silent can find it.
//!
//! So the fault is in the trace and not in the completion.
//! [`crate::proto::wrote::LINKDOWN`] appears where a frame went nowhere, and the
//! client is told the same thing it would have been told if the frame had
//! arrived. [`tests::a_client_cannot_tell_a_lost_frame_from_a_delivered_one`] is
//! that stated as an assertion, because it reads like a bug and is not.
//!
//! # Transmit only, and what that costs
//!
//! A real interface has two queues: the driver posts empty buffers on the
//! receive queue and the device fills them when something arrives. That needs a
//! client with a second submission path — one that offers memory without asking
//! for anything — and the ring protocol above this crate's client is a
//! request-and-answer one. Adding it would change the client, and the client is
//! the thing component substitution is a claim about.
//!
//! The sentence above is kept as it was written and is now known to be wrong in
//! its premise: a receive does **not** need a submission path that offers memory
//! without asking for anything. `user/virtio-net` posts a receive as an ordinary
//! request carrying an ordinary token, and answers it late. See the reversal
//! below.
//!
//! *Reversal:* the first component that receives rather than transmits.
//!
//! **It has arrived, at E1-B03 rather than at E2, and the prediction this
//! paragraph used to make was wrong in two specifics.** `user/virtio-net`
//! receives, and RFC 0051 records what it took. The corrections are worth
//! keeping because they are the reason a reversal condition is written down at
//! all:
//!
//! - This paragraph said *what changes then is the client*. The client did not
//!   change. `f_ring::buffers` was used unmodified: an `Idle` is moved into the
//!   posting entry and comes back from the completion, exactly as it does for a
//!   transmit, and RFC 0024's typestate turned out to express the receive
//!   direction without an edit.
//! - It said the driver gains *a path that posts buffers with no outstanding
//!   token*. It posts them **with** one. The ring protocol above the driver
//!   stays request-and-answer, and the token is what RFC 0024 requires to return
//!   the buffer — a tokenless post would leave a client holding an `InFlight`
//!   that no completion could ever answer.
//!
//! What is actually needed here is unchanged in shape and is not done: `serve`
//! gains a receive queue, this file gains a device that writes rather than
//! reads, and the fault classes and snapshot tags gain the cases that go with
//! it. That is its own task. Until then the model covers the transmit half of a
//! protocol whose interesting half is the other one, and saying so is better
//! than a reversal condition that describes a future which has already
//! happened.

use crate::dev::{Bus, Protocol, Request, Served};
use crate::fault::Class;
use crate::proto::wrote;
use crate::virtq::{Chain, Part, Region};

/// Bytes in the header every frame carries. Unit: bytes.
///
/// Twelve, which is the modern layout: a device that negotiated
/// `VIRTIO_F_VERSION_1` always uses the twelve-byte header with `num_buffers`,
/// where a legacy one used ten. The same fact `dma.rs` records for the block
/// device from the other end — the legacy interface is a different protocol
/// wearing one name, and a model that split the difference would be a model of
/// neither.
const HEADER_BYTES: u32 = 12;

/// Control memory one frame needs. Unit: bytes.
///
/// Sixteen rather than twelve, so that each slot's header starts eight-byte
/// aligned. Nothing in the header is eight bytes wide, and the alignment is kept
/// anyway because a layout whose alignment depends on the fields that happen to
/// be in it is a layout that breaks when a field is added.
const CONTROL_BYTES: u32 = 16;

/// No offload of any kind: the driver hands over a frame the device sends as it
/// is.
const GSO_NONE: u8 = 0;

/// Where the segmentation type sits in the header. Unit: bytes.
const GSO_AT: u32 = 1;

/// Where the buffer count sits. Unit: bytes.
const BUFFERS_AT: u32 = 10;

/// The network interface model.
#[derive(Clone, Copy, Debug, Default)]
pub struct Net;

impl Protocol for Net {
    const NAME: &'static str = "net";
    const TAG: u32 = crate::snap::tag::NET;
    const COMPLETE: &'static str = "net.complete";
    const DROP: &'static str = "net.drop";
    const COALESCE: &'static str = "net.coalesce";
    // A translation only. This protocol writes nothing back into control
    // memory — `harvest` says a network interface answers nothing at all —
    // so there is no last write for a partial one to lose, and claiming the
    // class would be claiming a site nothing exercises.
    const HONOURS: &'static [Class] = &[Class::MapFault];

    fn control_bytes(&self) -> u32 {
        CONTROL_BYTES
    }

    fn describe(
        &mut self,
        request: &Request,
        control: &mut Region,
        at: u32,
    ) -> Result<Vec<Part>, i32> {
        // Flags, segmentation type, header length, segment size, checksum start
        // and offset: all zero, which is a frame the driver has done nothing to
        // and the device must send whole. `num_buffers` is one, because this
        // driver never splits a frame across descriptors.
        control.put16(at, 0)?;
        control.put8(at + GSO_AT, GSO_NONE)?;
        control.put16(at + 2, 0)?;
        control.put16(at + 4, 0)?;
        control.put16(at + 6, 0)?;
        control.put16(at + 8, 0)?;
        control.put16(at + BUFFERS_AT, 1)?;

        Ok(vec![
            Part { at: control.device_at(at)?, len: HEADER_BYTES, write: false },
            // Both descriptors are device-*read*: this is a transmit, and a
            // transmit queue has nothing the device writes. A driver that
            // marked either of them writable would be a driver that expects an
            // answer where the protocol has none.
            Part { at: request.reach.address, len: request.reach.len, write: false },
        ])
    }

    fn serve(&mut self, chain: &Chain, bus: &mut Bus<'_>, extent: u64) -> Served {
        let [header, frame] = match chain.parts.as_slice() {
            [header, frame] => [*header, *frame],
            _ => return Served { used_len: 0, label: wrote::UNSUPP, fenced: false },
        };
        let Some(head_at) = bus.control_at(header.at, HEADER_BYTES) else {
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        };
        if header.write || frame.write || header.len != HEADER_BYTES {
            return Served { used_len: 0, label: wrote::UNSUPP, fenced: false };
        }
        if !bus.granted(frame.at, frame.len) {
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        }
        // An offload this device does not implement. Refused rather than sent
        // unsegmented, because a frame sent whole where the driver asked for
        // segmentation is a frame the far end will not reassemble — R04's case
        // exactly: a bit silently dropped is two peers with different beliefs
        // about what just happened.
        if bus.control.get8(head_at + GSO_AT) != Ok(GSO_NONE) {
            return Served { used_len: 0, label: wrote::UNSUPP, fenced: false };
        }

        // `extent` is the largest frame the link carries, in bytes, **and zero
        // is the link being down**. One field with a documented zero rather than
        // two, because the two are the same question asked of the wire: what can
        // it carry.
        let carried = u64::from(HEADER_BYTES.saturating_add(frame.len));
        let label = if extent == 0 {
            wrote::LINKDOWN
        } else if carried > extent {
            // Too long for the link. Dropped, and — this is the point of the
            // whole device — dropped *silently*, because a transmit queue has
            // nowhere to say so.
            wrote::LINKDOWN
        } else {
            wrote::SERVED
        };

        // Zero, always. Not an omission: a used entry on a transmit queue
        // reports the bytes the *device* wrote, and on this queue the device
        // writes nothing at all.
        Served { used_len: 0, label, fenced: false }
    }

    fn harvest(&mut self, _written: u32, _control: &Region, _at: u32, asked: u32) -> i32 {
        // The driver reports what it handed over, because that is the only thing
        // it knows. Reading the used length instead would report zero bytes for
        // every frame, which is a driver that mistakes *the device wrote nothing*
        // for *nothing was sent*.
        i32::try_from(asked).unwrap_or(i32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::CONTROL_BASE;
    use crate::service::Grants;
    use f_ring::registry::{Domains, Reach};

    fn bench() -> (Region, Grants, Reach) {
        let control = Region::new(CONTROL_BYTES * 4, CONTROL_BASE);
        let mut grants = Grants::new(4);
        let address = grants.map(0, 4096).expect("a domain with room");
        (control, grants, Reach { address, len: 1_000 })
    }

    fn served(extent: u64) -> (Served, i32) {
        let (mut control, grants, reach) = bench();
        let mut net = Net;
        let parts = net
            .describe(&Request { token: 1, at: 0, reach, seq: 0 }, &mut control, 0)
            .expect("a legal frame");
        let chain = Chain { head: 0, parts };
        let mut bus = Bus::new(&mut control, &grants);
        let out = net.serve(&chain, &mut bus, extent);
        let result = net.harvest(out.used_len, &control, 0, reach.len);
        (out, result)
    }

    #[test]
    fn a_client_cannot_tell_a_lost_frame_from_a_delivered_one() {
        // It reads like a bug and it is the protocol. A transmit queue has no
        // status byte, no response header and a used length of zero, so every
        // outcome the link can produce reaches the driver as the same
        // completion. The trace is where the difference lives, which is exactly
        // what makes this device worth modelling beside the block one: a client
        // that expects to be told about a drop has a bug only silence can find.
        let (sent, sent_result) = served(1_500);
        let (down, down_result) = served(0);
        let (long, long_result) = served(64);

        assert_eq!(sent.label, wrote::SERVED);
        assert_eq!(down.label, wrote::LINKDOWN);
        assert_eq!(long.label, wrote::LINKDOWN, "a frame past the link's size was carried");

        assert_eq!(sent.used_len, 0, "a transmit queue reported bytes the device wrote");
        assert_eq!(down.used_len, 0);
        assert_eq!(long.used_len, 0);
        assert_eq!(sent_result, 1_000);
        assert_eq!(down_result, sent_result, "the client was told a frame was lost");
        assert_eq!(long_result, sent_result);
    }

    #[test]
    fn the_link_carries_a_frame_of_exactly_its_size_and_not_one_byte_more() {
        // Header plus payload, on the boundary. An off-by-one here is a model
        // whose link is a byte wider than it says, which is the kind of
        // disagreement between a model and a device that produces a bug report
        // about the wrong thing.
        let (mut control, grants, reach) = bench();
        let mut net = Net;
        let exact = u64::from(HEADER_BYTES) + u64::from(reach.len);
        for (extent, label) in
            [(exact, wrote::SERVED), (exact + 1, wrote::SERVED), (exact - 1, wrote::LINKDOWN)]
        {
            let parts = net
                .describe(&Request { token: 1, at: 0, reach, seq: 0 }, &mut control, 0)
                .expect("a legal frame");
            let chain = Chain { head: 0, parts };
            let mut bus = Bus::new(&mut control, &grants);
            assert_eq!(
                net.serve(&chain, &mut bus, extent).label,
                label,
                "at an extent of {extent}"
            );
        }
    }

    #[test]
    fn an_offload_this_device_does_not_implement_is_refused() {
        // Written into the header directly, because no driver here emits one.
        // A frame sent whole where the driver asked for segmentation is a frame
        // the far end cannot reassemble, and R04 says a bit this build does not
        // know is refused rather than dropped.
        let (mut control, grants, reach) = bench();
        let mut net = Net;
        let parts = net
            .describe(&Request { token: 1, at: 0, reach, seq: 0 }, &mut control, 0)
            .expect("a legal frame");
        control.put8(GSO_AT, 4).expect("the header this test just wrote");
        let chain = Chain { head: 0, parts };
        let mut bus = Bus::new(&mut control, &grants);
        assert_eq!(net.serve(&chain, &mut bus, 1_500).label, wrote::UNSUPP);
    }

    #[test]
    fn a_descriptor_the_device_would_write_is_not_a_transmit() {
        // Both descriptors on a transmit queue are device-read. One the device
        // may write is a driver expecting an answer where the protocol has none,
        // and letting it through would be the model teaching a client a habit
        // the hardware will not honour.
        let (mut control, grants, reach) = bench();
        let mut net = Net;
        let mut parts = net
            .describe(&Request { token: 1, at: 0, reach, seq: 0 }, &mut control, 0)
            .expect("a legal frame");
        if let Some(frame) = parts.get_mut(1) {
            frame.write = true;
        }
        let chain = Chain { head: 0, parts };
        let mut bus = Bus::new(&mut control, &grants);
        assert_eq!(net.serve(&chain, &mut bus, 1_500).label, wrote::UNSUPP);
    }
}
