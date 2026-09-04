// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A display controller, as its driver sees it: a control header in, a response
//! header out, and a fence that says *this one may not be overtaken*.
//!
//! # Why a third device, and why this one
//!
//! The block device is free to complete in any order and the network interface
//! answers nothing at all. Neither of them can express the third thing a real
//! device does: **some of its completions are ordered and the rest are not.**
//! virtio-gpu's fences are that, and they are the reason this model is here
//! rather than a second block device with different constants.
//!
//! A request carrying [`FLAG_FENCE`] may not be published while an earlier
//! fenced request is still unpublished; everything without one is a free choice
//! the seed makes. A model that ordered everything would hide a driver that
//! forgot to fence, and a model that ordered nothing would report failures a
//! real device cannot produce. The rule itself lives in
//! [`crate::dev::Device::publishable`], because it is a statement about ordering
//! and ordering is that file's subject; what lives here is which requests carry
//! one.
//!
//! # What the driver emits, and why it never frees
//!
//! The client above the ring submits an operation on a buffer. This driver turns
//! each into one of two display commands, alternating: a resource creation, then
//! a transfer into the resource it just created. It emits no `RESOURCE_UNREF`,
//! and that is deliberate rather than missing — a driver that freed as it went
//! would never reach the display's limit, and the limit is the refusal worth
//! modelling. A real driver frees when its client drops the surface, which is a
//! lifetime this model has no client for; `Config::extent` is how many resources
//! the display holds, and running into it is the point.

use f_abi::error;

use crate::dev::{Bus, Protocol, Request, Served};
use crate::fault::Class;
use crate::proto::wrote;
use crate::virtq::{Chain, Part, Region};

/// Bytes in a control header: type, flags, fence id, context, ring index and
/// padding. Unit: bytes.
const HEADER_BYTES: u32 = 24;

/// Control memory one request needs: the header in, then the header out.
/// Unit: bytes.
const CONTROL_BYTES: u32 = 48;

/// Where the response header sits in a request's control slot. Unit: bytes.
const RESPONSE_AT: u32 = HEADER_BYTES;

/// Where the flags sit in a header. Unit: bytes.
const FLAGS_AT: u32 = 4;

/// Where the fence identifier sits. Unit: bytes.
const FENCE_AT: u32 = 8;

/// Where the context identifier sits — reused here as the resource this request
/// concerns, which is what the two commands below take. Unit: bytes.
const RESOURCE_AT: u32 = 16;

/// This request's completion may not be overtaken by a later fenced one.
const FLAG_FENCE: u32 = 1;

/// Make a two-dimensional resource.
const CMD_CREATE_2D: u32 = 0x0101;

/// Copy a buffer into one that exists.
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;

/// It worked, and there is nothing to read back.
const RESP_OK_NODATA: u32 = 0x1100;

/// Something else went wrong. The catch-all a device gives for a request it
/// understood and could not honour.
const RESP_ERR_UNSPEC: u32 = 0x1200;

/// The display holds as many resources as it can.
///
/// `0x1201`, and it was `0x1202` until E1-B04 wrote the real driver beside this
/// model. `0x1202` is `VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID` in the
/// specification's enumeration — the error codes run consecutively from
/// `ERR_UNSPEC` at `0x1200` — so this model was answering *no such scanout* to a
/// display that had run out of room, and the scenario that reads the refusal was
/// reading a number that means something else.
///
/// Worth a paragraph rather than a silent fix, because it is what a second
/// implementation is *for*. Nothing in this crate could have caught it: the
/// tests here assert against this constant, so they agreed with the model
/// whatever it said, and `Protocol::harvest` passes the device's number through
/// to the client unchanged — which is right, and which is also why a wrong
/// number travels the whole way. `user/virtio-gpu` does not use this code at all
/// (it passes through whatever a real display answers), so what found the defect
/// was a person reading the two files side by side. RFC 0054.
const RESP_ERR_OUT_OF_MEMORY: u32 = 0x1201;

/// No resource by that name.
const RESP_ERR_INVALID_RESOURCE_ID: u32 = 0x1203;

/// Nothing has been written here. Not a response any device sends, for the same
/// reason `blk`'s status byte starts at `0xFF`: a driver has to be able to tell
/// *the device refused* from *the device never answered*.
const RESP_NONE: u32 = 0;

/// The display controller model.
#[derive(Clone, Debug, Default)]
pub struct Gpu {
    /// Resources the display holds, in creation order. Unit: none — resource
    /// identifiers.
    ///
    /// A `Vec` and not a set: the count is what `extent` bounds and the order is
    /// creation order, which is what a reader of a trace wants. Small by
    /// construction — a scenario that wanted thousands would be a scenario about
    /// something else.
    ///
    /// **The device's, not the driver's.** [`Gpu::asked`] is the driver's and
    /// the two are separated for the reason that paragraph gives.
    live: Vec<u32>,
    /// Resources the *driver* has asked the display to create.
    /// Unit: none — resource identifiers.
    ///
    /// # Why this is not [`Gpu::live`]
    ///
    /// Because a driver knows what it asked for and not what the display holds,
    /// and the two come apart in exactly two places. A creation the display
    /// refused is in `asked` and not in `live`, which is what makes the transfer
    /// after a full display a transfer into a resource that does not exist —
    /// the failure this model exists to produce. And a **restarted** driver has
    /// asked for nothing, which is the case E1-B04 found.
    ///
    /// Chaos kills an occupant and refills its place, and `spawner` says what
    /// that means: *the same function, called again, with no state carried over
    /// from the instance that died.* That is right for a driver and wrong for a
    /// device, and this model was the only `Protocol` in the crate with state to
    /// notice — `Net` and `Blk` are unit structs. With one field the model was
    /// destroying a display's resources every time its driver died, and then
    /// transferring into them: four refusals a client could not retry, on a
    /// scenario whose whole claim is that a client observes nothing but latency.
    ///
    /// Splitting the two fixes it in the direction that is true rather than the
    /// one that is convenient. The device's resources still die with the model
    /// — making them outlive it is a change to `spawner`'s contract and belongs
    /// to whoever owns that scenario — but the *driver* now behaves the way a
    /// restarted driver does: it holds no identifiers, so it creates rather than
    /// transferring into something it cannot name. `user/virtio-gpu` is the real
    /// one and does the same thing for the same reason, one frame at a time.
    /// RFC 0054.
    asked: Vec<u32>,
}

impl Protocol for Gpu {
    const NAME: &'static str = "gpu";
    const TAG: u32 = crate::snap::tag::GPU;
    const COMPLETE: &'static str = "gpu.complete";
    const DROP: &'static str = "gpu.drop";
    const COALESCE: &'static str = "gpu.coalesce";
    // A translation only, for now. This protocol does write an answer back —
    // the response header — so `Class::Partial` is modellable here and simply
    // is not modelled; it is listed when `serve` asks `writes_land` and a
    // scenario asserts what a display controller's client is told, and not
    // before, because a class named here that nothing reads is exactly the
    // unexercised site this list exists to prevent.
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
        // Alternating: create, then transfer into what was just created. The
        // creation is fenced and the transfer is not, which is the ordering a
        // driver actually needs — a transfer may finish whenever the device
        // likes, and a creation may not finish after a later creation, or the
        // driver would not know which resource identifier the display had run
        // out on.
        //
        // The alternation is read out of what this driver has **asked for**
        // rather than out of the sequence number, and [`Gpu::asked`] argues why
        // at length. On an uninterrupted run the two are the same thing — even
        // sequences create and odd ones transfer — and they come apart exactly
        // where a driver restarts: an occupant that has asked for nothing
        // creates, because it holds no identifier it could transfer into.
        let resource = resource_of(request.at);
        let creating = !self.asked.contains(&resource);
        if creating {
            self.asked.push(resource);
        }
        let kind = if creating { CMD_CREATE_2D } else { CMD_TRANSFER_TO_HOST_2D };
        let flags = if creating { FLAG_FENCE } else { 0 };

        control.put32(at, kind)?;
        control.put32(at + FLAGS_AT, flags)?;
        control.put64(at + FENCE_AT, request.at)?;
        control.put32(at + RESOURCE_AT, resource)?;
        control.put32(at + RESPONSE_AT, RESP_NONE)?;

        Ok(vec![
            Part { at: control.device_at(at)?, len: HEADER_BYTES, write: false },
            // The buffer: what the resource is made of on a creation, what is
            // copied into it on a transfer. Device-read in both cases — a
            // display controller reads from the guest and writes to a screen.
            Part { at: request.reach.address, len: request.reach.len, write: false },
            Part { at: control.device_at(at + RESPONSE_AT)?, len: HEADER_BYTES, write: true },
        ])
    }

    fn serve(&mut self, chain: &Chain, bus: &mut Bus<'_>, extent: u64) -> Served {
        let [header, payload, response] = match chain.parts.as_slice() {
            [header, payload, response] => [*header, *payload, *response],
            _ => return Served { used_len: 0, label: wrote::UNSUPP, fenced: false },
        };
        let Some(head_at) = bus.control_at(header.at, HEADER_BYTES) else {
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        };
        let Some(resp_at) = bus.control_at(response.at, HEADER_BYTES) else {
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        };
        if header.write || payload.write || !response.write {
            return Served { used_len: 0, label: wrote::UNSUPP, fenced: false };
        }
        if !bus.granted(payload.at, payload.len) {
            return Served { used_len: 0, label: wrote::NOREACH, fenced: false };
        }

        let (Ok(kind), Ok(flags), Ok(fence), Ok(resource)) = (
            bus.control.get32(head_at),
            bus.control.get32(head_at + FLAGS_AT),
            bus.control.get64(head_at + FENCE_AT),
            bus.control.get32(head_at + RESOURCE_AT),
        ) else {
            return Served { used_len: 0, label: wrote::UNSUPP, fenced: false };
        };
        let fenced = flags & FLAG_FENCE != 0;

        let answer = match kind {
            CMD_CREATE_2D => {
                if self.live.contains(&resource) {
                    // The same identifier twice. A display that quietly replaced
                    // the first would leak whatever the first was made of, so it
                    // refuses — and the driver hears about it, which is what a
                    // resource identifier is for.
                    RESP_ERR_UNSPEC
                } else if u64::try_from(self.live.len()).unwrap_or(u64::MAX) >= extent {
                    RESP_ERR_OUT_OF_MEMORY
                } else {
                    self.live.push(resource);
                    RESP_OK_NODATA
                }
            }
            CMD_TRANSFER_TO_HOST_2D => {
                if self.live.contains(&resource) {
                    RESP_OK_NODATA
                } else {
                    // Reachable, and reachable for a reason worth having: the
                    // creation this transfer depends on was refused, so the
                    // transfer names a resource that does not exist. A display
                    // that copied into it anyway would be writing into memory
                    // nobody allocated.
                    RESP_ERR_INVALID_RESOURCE_ID
                }
            }
            _ => RESP_ERR_UNSPEC,
        };

        // The response header, written where the driver will read it: the type,
        // then the flags, then the fence identifier echoed back. Echoing the
        // fence is what makes a fence checkable at all — a driver that could not
        // tell which fence a response carried could not tell whether the device
        // honoured the order.
        let wrote_response = bus.control.put32(resp_at, answer).is_ok()
            && bus.control.put32(resp_at + FLAGS_AT, flags).is_ok()
            && bus.control.put64(resp_at + FENCE_AT, fence).is_ok();

        let label = match answer {
            _ if !wrote_response => wrote::NOREACH,
            RESP_OK_NODATA if fenced => wrote::FENCED,
            RESP_OK_NODATA => wrote::SERVED,
            _ => wrote::IOERR,
        };
        Served { used_len: if wrote_response { HEADER_BYTES } else { 0 }, label, fenced }
    }

    /// The resources the display holds, in creation order.
    ///
    /// The one protocol in this crate with state of its own, which is why
    /// [`Protocol::save_state`] exists as a pair of methods rather than as a
    /// silence. A snapshot that dropped this would restore a display with room
    /// it does not have, and the next creation would succeed where the real run
    /// refused it — a divergence that looks like a working device.
    fn save_state(&self, out: &mut crate::snap::Writer) {
        out.count(self.live.len());
        for resource in &self.live {
            out.u32(*resource);
        }
        // The driver's half, saved beside the device's rather than derived from
        // it. A restore that rebuilt `asked` from `live` would hand the restored
        // driver a record of a creation the display refused, and the next
        // request would transfer into a resource that does not exist where the
        // original run created one — which is a divergence that looks like a
        // display running out of room. RFC 0054.
        out.count(self.asked.len());
        for resource in &self.asked {
            out.u32(*resource);
        }
    }

    fn load_state(&mut self, input: &mut crate::snap::Reader<'_>) {
        let count = input.count(4, "more display resources than the file could hold");
        self.live = Vec::with_capacity(count);
        for _ in 0..count {
            self.live.push(input.u32());
        }
        let count = input.count(4, "more display requests than the file could hold");
        self.asked = Vec::with_capacity(count);
        for _ in 0..count {
            self.asked.push(input.u32());
        }
    }

    fn harvest(&mut self, _written: u32, control: &Region, at: u32, asked: u32) -> i32 {
        // The driver reads the response the device wrote, and reads the request
        // back out of the same slot to know what it asked for: a creation
        // transfers nothing and a transfer transfers the buffer. Both come out
        // of shared memory rather than out of a value carried alongside, which
        // is the round trip this model is for.
        let (Ok(answer), Ok(kind)) = (control.get32(at + RESPONSE_AT), control.get32(at)) else {
            return error::pack(error::DEVICE, 0);
        };
        match answer {
            RESP_OK_NODATA if kind == CMD_CREATE_2D => 0,
            RESP_OK_NODATA => i32::try_from(asked).unwrap_or(i32::MAX),
            // The response codes are sixteen bits wide in practice — `0x1200`
            // upward — so they fit `error`'s code field with room to spare, and
            // passing the device's own number through unchanged is R07: a
            // refusal the model invented a code for is a refusal the client
            // cannot act on.
            other => error::pack(error::DEVICE, u16::try_from(other).unwrap_or(u16::MAX)),
        }
    }
}

/// Which resource the request at sequence `at` concerns.
///
/// A creation and the transfer that follows it name the same one, which is what
/// makes the pair a pair. Counting from one, so that zero is never a resource —
/// a display that treated a zeroed header as naming resource zero would answer a
/// request nobody made.
const fn resource_of(at: u64) -> u32 {
    ((at / 2) as u32).wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::CONTROL_BASE;
    use crate::service::Grants;
    use f_ring::registry::{Domains, Reach};

    fn bench() -> (Region, Grants, Reach) {
        let control = Region::new(CONTROL_BYTES * 8, CONTROL_BASE);
        let mut grants = Grants::new(4);
        let address = grants.map(0, 4096).expect("a domain with room");
        (control, grants, Reach { address, len: 256 })
    }

    /// Run the requests at sequences `0..n` through one display, in order, and
    /// answer what each one earned.
    fn sequence(n: u64, extent: u64) -> Vec<(&'static str, i32, bool)> {
        let (mut control, grants, reach) = bench();
        let mut gpu = Gpu::default();
        let mut out = Vec::new();
        for at in 0..n {
            // One control slot per request, so the round trip is a real one:
            // reusing a slot would let a later request read an earlier one's
            // response.
            let slot = u32::try_from(at).unwrap_or(0).saturating_mul(CONTROL_BYTES);
            let parts = gpu
                .describe(&Request { token: at, at, reach, seq: at }, &mut control, slot)
                .expect("a legal request");
            let chain = Chain { head: 0, parts };
            let mut bus = Bus::new(&mut control, &grants);
            let served = gpu.serve(&chain, &mut bus, extent);
            let result = gpu.harvest(served.used_len, &control, slot, reach.len);
            out.push((served.label, result, served.fenced));
        }
        out
    }

    #[test]
    fn a_creation_is_fenced_and_a_transfer_is_not() {
        // The property the whole device exists for. If both were fenced the
        // model would order everything and hide a driver that forgot one; if
        // neither were, it would report failures a real display cannot produce.
        let seen = sequence(4, 8);
        assert_eq!(seen.len(), 4);
        assert_eq!(seen.first().map(|s| (s.0, s.2)), Some((wrote::FENCED, true)));
        assert_eq!(seen.get(1).map(|s| (s.0, s.2)), Some((wrote::SERVED, false)));
        assert_eq!(seen.get(2).map(|s| (s.0, s.2)), Some((wrote::FENCED, true)));
        assert_eq!(seen.get(3).map(|s| (s.0, s.2)), Some((wrote::SERVED, false)));
    }

    #[test]
    fn a_creation_transfers_nothing_and_a_transfer_transfers_the_buffer() {
        // What the client is told, which is not what the used ring says: both
        // requests write twenty-four bytes of response and only one of them is
        // a transfer. A model that reported the used length to the client would
        // tell it every display command moved twenty-four bytes.
        let seen = sequence(2, 8);
        assert_eq!(seen.first().map(|s| s.1), Some(0));
        assert_eq!(seen.get(1).map(|s| s.1), Some(256));
    }

    #[test]
    fn a_display_that_is_full_refuses_and_the_transfer_after_it_has_nothing_to_copy_into() {
        // Two refusals, and the second is the interesting one: it is *caused* by
        // the first, which is the shape of the failures a device model is
        // supposed to produce. A display that let the transfer through would be
        // writing into a resource nobody allocated.
        let seen = sequence(6, 2);
        assert_eq!(seen.get(4).map(|s| s.0), Some(wrote::IOERR), "a third resource was created");
        assert_eq!(
            seen.get(4).map(|s| error::unpack(s.1)),
            Some(Some((error::DEVICE, RESP_ERR_OUT_OF_MEMORY as u16)))
        );
        assert_eq!(
            seen.get(5).map(|s| error::unpack(s.1)),
            Some(Some((error::DEVICE, RESP_ERR_INVALID_RESOURCE_ID as u16))),
            "a transfer into a resource that was never created was served"
        );
    }

    #[test]
    fn a_command_the_display_does_not_implement_is_refused() {
        let (mut control, grants, reach) = bench();
        let mut gpu = Gpu::default();
        let parts = gpu
            .describe(&Request { token: 0, at: 0, reach, seq: 0 }, &mut control, 0)
            .expect("a legal request");
        control.put32(0, 0x0999).expect("the header this test just wrote");
        let chain = Chain { head: 0, parts };
        let mut bus = Bus::new(&mut control, &grants);
        assert_eq!(gpu.serve(&chain, &mut bus, 8).label, wrote::IOERR);
        assert_eq!(
            error::unpack(gpu.harvest(HEADER_BYTES, &control, 0, reach.len)),
            Some((error::DEVICE, RESP_ERR_UNSPEC as u16))
        );
    }

    #[test]
    fn a_response_nothing_wrote_is_not_an_answer() {
        // `blk`'s `0xFF` argument, in the display's own vocabulary. Zero is not
        // a response type any device sends, so a driver reading one has read a
        // slot the device never touched — and calling that success would report
        // every unanswered request as a completed one.
        let (control, _grants, _reach) = bench();
        let mut gpu = Gpu::default();
        assert_eq!(
            error::unpack(gpu.harvest(0, &control, 0, 256)),
            Some((error::DEVICE, RESP_NONE as u16)),
            "an untouched response slot was read as an answer"
        );
    }
}
