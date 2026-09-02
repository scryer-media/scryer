//! The in-memory seam between gotatun and smoltcp.
//!
//! gotatun is written against a TUN device, expressed as the pair of traits
//! [`IpSend`] (device → operating system) and [`IpRecv`] (operating system →
//! device). Nothing about those traits requires an operating system, so this
//! module supplies the pair backed by two bounded tokio channels and the
//! device never learns that the "kernel" on the other side is a smoltcp
//! interface running in the same process. That is the whole reason no `tun`
//! feature, no interface and no elevated privileges are needed.
//!
//! ```text
//!   smoltcp Interface ──▶ [StackToDevice] ──▶ gotatun encrypt ──▶ UDP
//!   smoltcp Interface ◀── [DeviceToStack] ◀── gotatun decrypt ◀── UDP
//! ```
//!
//! ## Why not `gotatun::tun::channel`
//!
//! gotatun ships `TunChannelTx`/`TunChannelRx`, which look like exactly this.
//! They are not: their channels carry `Packet<Ipv4<Udp>>`/`Packet<Ipv6<Udp>>`,
//! so they can only carry **UDP**. They exist for gotatun's own
//! WireGuard-inside-WireGuard transport, and a TCP segment cannot travel
//! through them at all. Our channels carry `Packet<Ip>`, the untyped IP packet
//! the traits are actually defined over.
//!
//! ## Copies
//!
//! One write per direction and no copies. Outbound, smoltcp composes its
//! packet directly into a buffer taken from a [`PacketBufPool`], which is then
//! moved into the channel. Inbound, the `Packet<Ip>` gotatun decrypted into is
//! moved through the channel and handed to smoltcp as a borrowed slice.

use std::io;

use gotatun::packet::{Ip, Packet, PacketBufPool};
use gotatun::tun::{IpRecv, IpSend, MtuWatcher};
use tokio::sync::mpsc;

/// How many packets may be in flight in either direction before the producer
/// feels back-pressure.
///
/// A tunnel is a link, and links have queues. This one is deliberately small:
/// it is one bandwidth-delay product's worth of a slow link, which is enough
/// that a burst is absorbed and small enough that a stalled reader turns into
/// loss (which TCP handles) rather than unbounded memory (which it does not).
pub(crate) const PACKET_QUEUE_DEPTH: usize = 512;

/// Build the two channels that join one smoltcp stack to one gotatun device.
///
/// Returns, in order: the [`IpSend`] and [`IpRecv`] halves to hand to
/// `DeviceBuilder::with_ip_pair`, the receiver the stack pump drains for
/// inbound packets, and the sender the stack's `phy::Device` writes outbound
/// packets to.
pub(crate) fn ip_channels(mtu: u16) -> (DeviceToStack, StackToDevice, InboundRx, OutboundTx) {
    let (inbound_tx, inbound_rx) = mpsc::channel(PACKET_QUEUE_DEPTH);
    let (outbound_tx, outbound_rx) = mpsc::channel(PACKET_QUEUE_DEPTH);
    (
        DeviceToStack { tx: inbound_tx },
        StackToDevice {
            rx: outbound_rx,
            batch: Vec::new(),
            mtu,
        },
        inbound_rx,
        outbound_tx,
    )
}

/// Packets gotatun decrypted, on their way to the smoltcp stack.
pub(crate) type InboundRx = mpsc::Receiver<Packet<Ip>>;

/// Packets smoltcp emitted, on their way to gotatun for encryption.
pub(crate) type OutboundTx = mpsc::Sender<Packet<Ip>>;

/// The [`IpSend`] half: what gotatun writes decrypted packets into.
pub(crate) struct DeviceToStack {
    tx: mpsc::Sender<Packet<Ip>>,
}

impl IpSend for DeviceToStack {
    async fn send(&mut self, packet: Packet<Ip>) -> io::Result<()> {
        // A closed channel means the stack pump is gone, which is how a
        // tunnel shuts down. gotatun treats `BrokenPipe` as fatal and unwinds
        // its own tasks, which is exactly what we want: dropping the pump
        // tears the device down with it.
        self.tx
            .send(packet)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the tunnel stack has stopped"))
    }
}

/// The [`IpRecv`] half: what gotatun reads packets to encrypt from.
pub(crate) struct StackToDevice {
    rx: mpsc::Receiver<Packet<Ip>>,
    /// Reused across calls so a batch of packets costs no allocation.
    batch: Vec<Packet<Ip>>,
    mtu: u16,
}

impl IpRecv for StackToDevice {
    async fn recv<'a>(
        &'a mut self,
        _pool: &mut PacketBufPool,
    ) -> io::Result<impl Iterator<Item = Packet<Ip>> + Send + 'a> {
        // The pool argument is gotatun offering us buffers to read into. We
        // have nothing to read *into*: the packets already exist, composed by
        // smoltcp into buffers from our own pool, so we hand them straight
        // over. Draining in batches keeps one wakeup per burst rather than
        // one per packet.
        let wanted = self.rx.max_capacity();
        let taken = self.rx.recv_many(&mut self.batch, wanted).await;
        if taken == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the tunnel stack has stopped",
            ));
        }
        Ok(self.batch.drain(..taken))
    }

    fn mtu(&self) -> MtuWatcher {
        // Constant: this link's MTU is whatever the operator configured, and
        // nothing can change it underneath us the way an OS interface can.
        MtuWatcher::new(self.mtu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gotatun::packet::{IpNextProtocol, Ipv4Header};
    use std::net::Ipv4Addr;

    /// The smallest thing gotatun will accept as an IP packet.
    fn packet(payload: &[u8]) -> Packet<Ip> {
        let header = Ipv4Header::new(
            Ipv4Addr::new(10, 6, 0, 2),
            Ipv4Addr::new(10, 6, 0, 1),
            IpNextProtocol::Udp,
            payload,
        );
        let mut packet = Packet::copy_from(&header).into_bytes();
        packet.buf_mut().extend_from_slice(payload);
        packet.try_into_ip().expect("a valid IP packet")
    }

    #[tokio::test]
    async fn packets_cross_in_both_directions() {
        let (mut to_stack, mut to_device, mut inbound, outbound) = ip_channels(1280);

        to_stack.send(packet(b"inbound")).await.expect("send");
        let received = inbound.recv().await.expect("inbound packet");
        assert!(
            Packet::<[u8]>::from(received)
                .as_ref()
                .ends_with(b"inbound"),
            "the packet must arrive intact"
        );

        outbound.send(packet(b"outbound")).await.expect("queue");
        let mut pool = PacketBufPool::new(1);
        let drained: Vec<_> = to_device.recv(&mut pool).await.expect("recv").collect();
        assert_eq!(drained.len(), 1);
        assert!(
            Packet::<[u8]>::from(drained.into_iter().next().expect("packet"))
                .as_ref()
                .ends_with(b"outbound")
        );
    }

    #[tokio::test]
    async fn a_burst_is_drained_in_one_batch() {
        let (_to_stack, mut to_device, _inbound, outbound) = ip_channels(1280);
        for _ in 0..8 {
            outbound.send(packet(b"burst")).await.expect("queue");
        }
        let mut pool = PacketBufPool::new(1);
        let drained: Vec<_> = to_device.recv(&mut pool).await.expect("recv").collect();
        assert_eq!(drained.len(), 8, "one wakeup must drain the whole burst");
    }

    #[tokio::test]
    async fn a_dropped_stack_ends_both_halves() {
        let (mut to_stack, mut to_device, inbound, outbound) = ip_channels(1280);

        drop(inbound);
        let error = to_stack.send(packet(b"nowhere")).await.expect_err("closed");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);

        drop(outbound);
        let mut pool = PacketBufPool::new(1);
        let error = to_device.recv(&mut pool).await.err().expect("closed");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn the_mtu_is_the_configured_one() {
        let (_to_stack, to_device, _inbound, _outbound) = ip_channels(1420);
        let mut mtu = to_device.mtu();
        assert_eq!(mtu.get(), 1420);
    }
}
