// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A block device, as its driver sees it: a three-descriptor chain, a status
//! byte, and completions in whatever order the device likes.
//!
//! # Why this one is the detailed one
//!
//! `E1-P10`'s datapath claims and `E1-B14`'s unmap-under-churn workload are both
//! block workloads, and both need a device that behaves like a disk rather than
//! like a queue with a delay. So the request layout here is virtio-blk's, field
//! for field, and the four refusals below are the ones a real device gives.
//!
//! The reference is `kernel/src/arch/x86_64/dma.rs`, which builds exactly this
//! chain by hand against a real QEMU device: the header the device reads, the
//! sector it writes, and the status byte it writes last. What that file
//! establishes and this one inherits is the shape — three descriptors, sixteen
//! bytes of header, one byte of status — and the discipline of writing `0xFF`
//! into the status byte first, because `0xFF` is not a status any device defines
//! and so a byte still holding it afterwards means *nothing was written here*
//! rather than *the device reported something*.
//!
//! # The two-way trip through shared memory
//!
//! [`Blk::describe`] writes a header into the control region and the device
//! reads it back out of the same bytes in [`Blk::serve`]; the device writes a
//! status byte and the driver reads it back in [`Blk::harvest`]. Neither half
//! is handed the other's answer in a struct. That is more code than passing a
//! value along, and it is the only version of this model that could catch a
//! driver and a device disagreeing about where a field is — which is the whole
//! class of bug a protocol model exists to find.
//!
//! # What the driver decides and what the client does not
//!
//! The client above the ring submits *an operation on a buffer at an offset*.
//! Whether that is a read or a write is the driver's, and this one alternates:
//! even sequence numbers read, odd ones write. A model where every request went
//! the same way would never exercise the direction check, and a client that had
//! to state the direction would be a client that knew it was talking to a disk —
//! which is exactly the knowledge component substitution says it must not need.

use f_abi::error;

use crate::dev::{Bus, Protocol, Request, Served};
use crate::fault::Class;
use crate::proto::wrote;
use crate::virtq::{Part, Region};

/// A block read: the device writes the data buffer.
const BLK_IN: u32 = 0;

/// A block write: the device reads it.
const BLK_OUT: u32 = 1;

/// Bytes in the request header: type, priority, sector. Unit: bytes.
const HEADER_BYTES: u32 = 16;

/// Bytes in a sector. Unit: bytes.
///
/// The unit `Config::extent` counts for this device, and the reason the extent
/// is a sector count rather than a byte count: a disk's size is in sectors and a
/// request past the end is refused on a sector boundary.
const SECTOR_BYTES: u32 = 512;

/// Control memory one request needs: the header, then the status byte. Unit:
/// bytes.
///
/// Thirty-two rather than seventeen, so that every slot's header starts
/// eight-byte aligned and the sector field can be written as one `put64`. The
/// alternative is a packed layout and a misaligned write, which
/// [`Region::put64`] refuses — correctly, because a device reading a misaligned
/// field is a device the specification does not describe.
const CONTROL_BYTES: u32 = 32;

/// Where the status byte sits in a request's control slot. Unit: bytes.
const STATUS_AT: u32 = HEADER_BYTES;

/// The device did it.
const STATUS_OK: u8 = 0;

/// The device could not: a sector past the disk, or memory it cannot reach.
const STATUS_IOERR: u8 = 1;

/// The device does not implement that request type.
const STATUS_UNSUPP: u8 = 2;

/// Not a status any device defines.
///
/// Written by the driver before the chain is offered, so that a byte still
/// holding it afterwards means the device wrote nothing — which is a different
/// claim from *the device reported a failure* and the only one that separates a
/// refused transfer from a transfer that never happened. `dma.rs` uses the same
/// value for the same reason.
const STATUS_NONE: u8 = 0xFF;

/// The block device model.
#[derive(Clone, Copy, Debug, Default)]
pub struct Blk;

impl Protocol for Blk {
    const NAME: &'static str = "blk";
    const COMPLETE: &'static str = "blk.complete";
    const DROP: &'static str = "blk.drop";
    const COALESCE: &'static str = "blk.coalesce";
    // Both bus classes: `serve` asks `granted` of the data buffer and
    // `writes_land` before the status byte, so a scenario may arm either
    // against this device and see it change the run.
    const HONOURS: &'static [Class] = &[Class::MapFault, Class::Partial];

    fn control_bytes(&self) -> u32 {
        CONTROL_BYTES
    }

    fn describe(
        &mut self,
        request: &Request,
        control: &mut Region,
        at: u32,
    ) -> Result<Vec<Part>, i32> {
        let reading = request.at.is_multiple_of(2);
        let kind = if reading { BLK_IN } else { BLK_OUT };

        // The sixteen bytes a block request header is, in the order the
        // specification fixes: type, then priority, then the sector. One
        // statement per field, as `dma.rs` writes them, because three fields in
        // one call would be three offsets a reader has to trust rather than
        // three a reader can check.
        control.put32(at, kind)?;
        control.put32(at + 4, 0)?;
        control.put64(at + 8, request.at)?;
        control.put8(at + STATUS_AT, STATUS_NONE)?;

        Ok(vec![
            Part { at: control.device_at(at)?, len: HEADER_BYTES, write: false },
            // The one descriptor whose address the driver did not choose: it is
            // the client's buffer, as the registration resolved it, and nothing
            // in this crate can turn it into bytes.
            Part { at: request.reach.address, len: request.reach.len, write: reading },
            Part { at: control.device_at(at + STATUS_AT)?, len: 1, write: true },
        ])
    }

    fn serve(&mut self, chain: &crate::virtq::Chain, bus: &mut Bus<'_>, extent: u64) -> Served {
        // Every check below is a driver that wrote something a real device
        // would refuse, and a model that accepted any of them would be excusing
        // a bug the hardware would not. R04: refused, never ignored.
        let [header, data, status] = match chain.parts.as_slice() {
            [header, data, status] => [*header, *data, *status],
            // A chain that is not three descriptors is not a block request.
            // There is nowhere to write a status byte, so the device answers
            // with a used entry of zero and the driver reads `STATUS_NONE` back
            // out of the byte it wrote itself — which is exactly what a real
            // device leaves behind when it cannot honour a chain.
            _ => return Served { used_len: 0, label: wrote::UNSUPP, fenced: false },
        };

        let Some(head_at) = bus.control_at(header.at, HEADER_BYTES) else {
            // The header is a descriptor the driver built out of its own
            // control region, so an address the device cannot decode means the
            // two disagree about where that region is. There is no status byte
            // to write into either, so this is the one refusal with nowhere to
            // put an answer.
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        };
        let Some(status_at) = bus.control_at(status.at, 1) else {
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        };
        if header.write || header.len != HEADER_BYTES || !status.write || status.len != 1 {
            return Self::answer(bus, status_at, STATUS_UNSUPP, 0);
        }

        let (Ok(kind), Ok(sector)) = (bus.control.get32(head_at), bus.control.get64(head_at + 8))
        else {
            return Self::answer(bus, status_at, STATUS_UNSUPP, 0);
        };
        let reading = match kind {
            BLK_IN => true,
            BLK_OUT => false,
            // An opcode this device does not implement. The refusal a real one
            // gives, and the reason the client is told `DEVICE` rather than
            // `ARGUMENT`: the entry was well formed and the *device* is what
            // declined it.
            _ => return Self::answer(bus, status_at, STATUS_UNSUPP, 0),
        };
        if data.write != reading {
            // A read whose data descriptor the device may not write, or a write
            // whose descriptor it may. Either way the driver and the header
            // disagree, which is the bug this round trip exists to catch.
            return Self::answer(bus, status_at, STATUS_UNSUPP, 0);
        }
        if !bus.granted(data.at, data.len) {
            // The address is outside every translation the component's domain
            // holds. On real silicon this is a fault the remapping unit raises
            // and `dma.rs` provokes on purpose; here it is a refusal, because a
            // model that let the device reach it would be a model of a machine
            // with no IOMMU.
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        }

        // The sector arithmetic, on a boundary rather than on a byte: a request
        // that starts inside the disk and ends outside it is refused whole,
        // because a device that served the readable half would report a short
        // transfer the driver has no field to express.
        let sectors = u64::from(data.len.div_ceil(SECTOR_BYTES));
        if sector.saturating_add(sectors) > extent {
            return Self::answer(bus, status_at, STATUS_IOERR, 0);
        }

        // The used length is what the *device* wrote: the payload and the
        // status byte on a read, the status byte alone on a write. A model that
        // reported the request length either way would be a model a driver
        // could not tell a read from a write with.
        let used_len = if reading { data.len.saturating_add(1) } else { 1 };

        // `E1-P02`'s partial write. The payload moved and the device's *last*
        // write — the status byte — did not, so the used entry says the transfer
        // happened and the byte the driver reads back is still the `0xFF` it
        // wrote itself. That is precisely the case `STATUS_NONE` exists to make
        // visible, and until this class there was nothing in any scenario that
        // reached it: a used length is not evidence that bytes moved, and
        // `harvest` below reads the byte rather than the length for that reason.
        //
        // Only the successful answer is torn. A refusal that lost its status
        // byte would be indistinguishable from this, and two causes with one
        // client-visible answer is a class that cannot be asserted about.
        if !bus.writes_land() {
            return Served { used_len, label: wrote::SERVED, fenced: false };
        }
        Self::answer(bus, status_at, STATUS_OK, used_len)
    }

    fn harvest(&mut self, _written: u32, control: &Region, at: u32, asked: u32) -> i32 {
        // The driver's answer comes out of the byte the device wrote, not out
        // of the used length. `dma.rs` records why: this emulator's block device
        // reports a successful completion for a transfer the remapping unit
        // refused, so a completion is evidence that the device finished and
        // never evidence that bytes moved. The status byte is the evidence.
        match control.get8(at + STATUS_AT) {
            Ok(STATUS_OK) => i32::try_from(asked).unwrap_or(i32::MAX),
            // Every other value, including the `STATUS_NONE` the driver wrote
            // itself, is a refusal carrying the device's own status — which is
            // what `error::DEVICE`'s detail is defined to be.
            Ok(status) => error::pack(error::DEVICE, u16::from(status)),
            Err(refused) => refused,
        }
    }
}

impl Blk {
    /// Write the status byte and answer what the used ring will say.
    ///
    /// One place, so that no refusal can forget to write a status: a chain the
    /// device took and answered nothing into is a chain whose driver waits
    /// forever on a byte that never changed.
    fn answer(bus: &mut Bus<'_>, status_at: u32, status: u8, payload: u32) -> Served {
        let wrote_status = bus.control.put8(status_at, status).is_ok();
        let used_len = if wrote_status { payload.max(1) } else { 0 };
        let label = match status {
            STATUS_OK => wrote::SERVED,
            STATUS_UNSUPP => wrote::UNSUPP,
            _ => wrote::IOERR,
        };
        Served { used_len, label, fenced: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::CONTROL_BASE;
    use crate::service::{GRANT_BASE, Grants};
    use crate::virtq::{Chain, Queue};
    use f_ring::registry::{Domains, Reach};

    /// A control region and a domain holding one grant, which is the smallest
    /// setting a request needs.
    fn bench() -> (Region, Grants, Reach) {
        let control = Region::new(CONTROL_BYTES * 4, CONTROL_BASE);
        let mut grants = Grants::new(4);
        let address = grants.map(0, 4096).expect("a domain with room");
        (control, grants, Reach { address, len: 512 })
    }

    fn request(at: u64, reach: Reach) -> Request {
        Request { token: 7, at, reach, seq: at }
    }

    /// Drive one request all the way round: describe, offer, take, serve,
    /// publish, harvest, read.
    fn round(at: u64) -> (Served, i32) {
        let (mut control, grants, reach) = bench();
        let mut blk = Blk;
        let mut queue = Queue::new(8, crate::dev::QUEUE_BASE).expect("a legal queue");
        let parts = blk.describe(&request(at, reach), &mut control, 0).expect("a legal request");
        let head = queue.chain(&parts).expect("three descriptors fit");
        queue.offer(head).expect("inside the ring");
        let chain = queue.take().expect("a legal ring").expect("the offered chain");

        let mut bus = Bus::new(&mut control, &grants);
        let served = blk.serve(&chain, &mut bus, 64);
        let result = blk.harvest(served.used_len, &control, 0, reach.len);
        (served, result)
    }

    #[test]
    fn a_read_and_a_write_round_trip_and_say_which_they_were() {
        // The whole two-way trip, and the one property that says the header
        // reached the device: a read's used length carries the payload and a
        // write's does not.
        let (read, read_result) = round(0);
        assert_eq!(read.label, wrote::SERVED);
        assert_eq!(read.used_len, 513, "a read reports its payload and its status byte");
        assert_eq!(read_result, 512);

        let (write, write_result) = round(1);
        assert_eq!(write.label, wrote::SERVED);
        assert_eq!(write.used_len, 1, "a write reports only its status byte");
        assert_eq!(write_result, 512);
    }

    #[test]
    fn a_sector_past_the_disk_is_refused_on_a_boundary() {
        // The disk in `round` is sixty-four sectors and the request is one
        // sector. Sector sixty-three is the last legal one, and sixty-four is
        // the first that is not — checked at the boundary, because an
        // off-by-one here is a device that serves a sector it does not have.
        let (mut control, grants, reach) = bench();
        let mut blk = Blk;
        for (sector, ok) in [(62u64, true), (63, true), (64, false), (1 << 40, false)] {
            let parts =
                blk.describe(&request(sector, reach), &mut control, 0).expect("a legal request");
            let chain = Chain { head: 0, parts };
            let mut bus = Bus::new(&mut control, &grants);
            let served = blk.serve(&chain, &mut bus, 64);
            let result = blk.harvest(served.used_len, &control, 0, reach.len);
            if ok {
                assert_eq!(served.label, wrote::SERVED, "sector {sector} was refused");
                assert!(result >= 0);
            } else {
                assert_eq!(served.label, wrote::IOERR, "sector {sector} was served");
                assert_eq!(
                    error::unpack(result),
                    Some((error::DEVICE, u16::from(STATUS_IOERR))),
                    "the driver read something other than the status the device wrote"
                );
            }
        }
    }

    #[test]
    fn an_address_the_domain_does_not_translate_is_refused() {
        // The model's stand-in for the fault `dma.rs` provokes. The data
        // descriptor points one grant along, at memory the component was never
        // given, and the device must not serve it.
        let (mut control, grants, _reach) = bench();
        let mut blk = Blk;
        let stray = Reach { address: GRANT_BASE + crate::service::GRANT_STRIDE, len: 512 };
        let parts = blk.describe(&request(0, stray), &mut control, 0).expect("a legal request");
        let chain = Chain { head: 0, parts };
        let mut bus = Bus::new(&mut control, &grants);
        let served = blk.serve(&chain, &mut bus, 64);
        assert_eq!(served.label, wrote::NOREACH);
        assert_eq!(served.used_len, 0, "a refused transfer reported bytes");
    }

    #[test]
    fn a_request_type_the_device_does_not_implement_is_refused() {
        // Written into the header directly, because no driver in this crate
        // emits one — which is the point: R04 is about what happens when a peer
        // writes something this build does not know, and the only way to test
        // that is to be that peer.
        let (mut control, grants, reach) = bench();
        let mut blk = Blk;
        let parts = blk.describe(&request(0, reach), &mut control, 0).expect("a legal request");
        control.put32(0, 9).expect("the header this test just wrote");
        let chain = Chain { head: 0, parts };
        let mut bus = Bus::new(&mut control, &grants);
        let served = blk.serve(&chain, &mut bus, 64);
        assert_eq!(served.label, wrote::UNSUPP);
        assert_eq!(
            error::unpack(blk.harvest(served.used_len, &control, 0, reach.len)),
            Some((error::DEVICE, u16::from(STATUS_UNSUPP)))
        );
    }

    #[test]
    fn a_chain_that_is_not_three_descriptors_is_not_a_block_request() {
        let (mut control, grants, reach) = bench();
        let mut blk = Blk;
        let full = blk.describe(&request(0, reach), &mut control, 0).expect("a legal request");
        for parts in [&full[..1], &full[..2]] {
            let chain = Chain { head: 0, parts: parts.to_vec() };
            let mut bus = Bus::new(&mut control, &grants);
            assert_eq!(blk.serve(&chain, &mut bus, 64).label, wrote::UNSUPP);
        }
    }

    #[test]
    fn a_status_byte_nothing_wrote_is_not_a_success() {
        // The `0xFF` trick, checked rather than assumed. If the driver read an
        // unwritten byte as success, every refusal above would be reported to
        // the client as a completed transfer — which is the failure mode this
        // whole model exists to be able to see.
        let (mut control, _grants, _reach) = bench();
        let mut blk = Blk;
        control.put8(STATUS_AT, STATUS_NONE).expect("inside the region");
        assert_eq!(
            error::unpack(blk.harvest(0, &control, 0, 512)),
            Some((error::DEVICE, u16::from(STATUS_NONE))),
            "an unwritten status byte was read as an answer"
        );
    }
}
