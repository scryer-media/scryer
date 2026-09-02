//! smoltcp's view of the tunnel: a `phy::Device` whose wire is the adapter.
//!
//! smoltcp asks a device for tokens rather than for buffers, which is exactly
//! the shape we want: the transmit token hands smoltcp a buffer taken from
//! gotatun's own [`PacketBufPool`], so the packet smoltcp composes *is* the
//! packet gotatun encrypts — composed once, never copied.
//!
//! The medium is [`Medium::Ip`]: a WireGuard tunnel is point-to-point, there
//! are no link-layer addresses, no ARP and no neighbour discovery. That is why
//! the interface's hardware address is [`HardwareAddress::Ip`] and why
//! smoltcp's neighbour cache never comes into play.

use std::collections::VecDeque;

use gotatun::packet::{Ip, Packet, PacketBufPool};
use smoltcp::phy::{self, ChecksumCapabilities, DeviceCapabilities, Medium};
use smoltcp::time::Instant;

use crate::wireguard::adapter::{OutboundTx, PACKET_QUEUE_DEPTH};

/// The smoltcp side of the link.
pub(crate) struct WgPhy {
    /// Decrypted packets waiting to be handed to the interface. Filled by the
    /// stack pump from the inbound channel, drained by `receive`.
    inbound: VecDeque<Packet<Ip>>,
    /// Where composed packets go to be encrypted.
    outbound: OutboundTx,
    /// Buffers for outbound packets. Sized to the in-flight depth of the
    /// outbound channel so a saturated link recycles rather than allocates.
    pool: PacketBufPool,
    mtu: usize,
    /// Outbound packets discarded because the link was congested. Traced, not
    /// surfaced: dropping under congestion is what a link does, and TCP is
    /// what notices.
    dropped: u64,
    /// Inbound packets discarded because the interface was not draining. Same
    /// reasoning, other direction.
    overrun: u64,
}

impl WgPhy {
    pub(crate) fn new(outbound: OutboundTx, mtu: u16) -> Self {
        Self {
            inbound: VecDeque::with_capacity(PACKET_QUEUE_DEPTH),
            outbound,
            pool: PacketBufPool::new(PACKET_QUEUE_DEPTH),
            mtu: usize::from(mtu),
            dropped: 0,
            overrun: 0,
        }
    }

    /// Queue a decrypted packet for the next `poll`.
    ///
    /// Bounded by the same depth as the channels: if the interface is not
    /// keeping up there is no point holding packets a sender has long since
    /// retransmitted, so the oldest is dropped.
    pub(crate) fn push_inbound(&mut self, packet: Packet<Ip>) {
        if self.inbound.len() >= PACKET_QUEUE_DEPTH {
            self.inbound.pop_front();
            self.overrun += 1;
        }
        self.inbound.push_back(packet);
    }

    /// Packets discarded in each direction, for tracing on teardown.
    pub(crate) fn drop_counts(&self) -> (u64, u64) {
        (self.dropped, self.overrun)
    }
}

impl phy::Device for WgPhy {
    type RxToken<'a> = WgRxToken;
    type TxToken<'a> = WgTxToken<'a>;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ip;
        capabilities.max_transmission_unit = self.mtu;
        // Checksums are computed and verified here rather than delegated:
        // there is no hardware to offload to, and the far side is a real
        // WireGuard peer that will drop anything malformed.
        capabilities.checksum = ChecksumCapabilities::default();
        capabilities
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let packet = self.inbound.pop_front()?;
        Some((WgRxToken { packet }, WgTxToken { device: self }))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        // The outbound channel is bounded; refusing a token while it is full
        // is how back-pressure reaches smoltcp, which will simply retry on
        // the next poll and leave the segment in the socket's send buffer.
        if self.outbound.capacity() == 0 {
            return None;
        }
        Some(WgTxToken { device: self })
    }
}

/// One decrypted packet, waiting to be parsed by the interface.
pub(crate) struct WgRxToken {
    packet: Packet<Ip>,
}

impl phy::RxToken for WgRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let bytes = Packet::<[u8]>::from(self.packet);
        f(bytes.as_ref())
    }
}

/// Permission to compose one packet straight into a pooled buffer.
pub(crate) struct WgTxToken<'a> {
    device: &'a mut WgPhy,
}

impl phy::TxToken for WgTxToken<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut packet = self.device.pool.get();
        if len > packet.len() {
            // Only reachable with an MTU above the pool's buffer size, which
            // `WireGuardSpec::validate` already refuses; handled rather than
            // asserted because a panic here would take the stack pump down.
            packet.buf_mut().resize(len, 0);
        } else {
            packet.truncate(len);
        }

        let result = f(&mut packet[..]);

        match packet.try_into_ip() {
            Ok(packet) => {
                if self.device.outbound.try_send(packet).is_err() {
                    // Full, or the device is gone. Either way this packet is
                    // lost, which is a link doing what links do.
                    self.device.dropped += 1;
                }
            }
            Err(error) => {
                // smoltcp does not emit malformed IP packets, so this would be
                // a bug rather than an operational condition.
                tracing::debug!(error = %error, "the tunnel stack composed an unparseable packet");
                self.device.dropped += 1;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wireguard::adapter::ip_channels;
    use gotatun::packet::{IpNextProtocol, Ipv4Header};
    use smoltcp::phy::{Device as _, RxToken as _, TxToken as _};
    use std::net::Ipv4Addr;

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
    async fn the_capabilities_describe_a_point_to_point_ip_link() {
        let (_to_stack, _to_device, _inbound, outbound) = ip_channels(1280);
        let phy = WgPhy::new(outbound, 1280);
        let capabilities = phy.capabilities();
        assert_eq!(capabilities.medium, Medium::Ip);
        assert_eq!(capabilities.max_transmission_unit, 1280);
    }

    #[tokio::test]
    async fn transmit_composes_into_the_outbound_channel() {
        let (_to_stack, mut to_device, _inbound, outbound) = ip_channels(1280);
        let mut phy = WgPhy::new(outbound, 1280);

        let bytes = Packet::<[u8]>::from(packet(b"hello")).as_ref().to_vec();
        let token = phy.transmit(Instant::from_micros(0)).expect("a tx token");
        token.consume(bytes.len(), |buffer| buffer.copy_from_slice(&bytes));

        let mut pool = PacketBufPool::new(1);
        let mut drained: Vec<_> = {
            use gotatun::tun::IpRecv as _;
            to_device.recv(&mut pool).await.expect("recv").collect()
        };
        assert_eq!(drained.len(), 1);
        let sent = Packet::<[u8]>::from(drained.pop().expect("packet"));
        assert_eq!(sent.as_ref(), bytes.as_slice());
        assert_eq!(phy.drop_counts(), (0, 0));
    }

    #[tokio::test]
    async fn receive_hands_the_interface_the_decrypted_bytes() {
        let (_to_stack, _to_device, _inbound, outbound) = ip_channels(1280);
        let mut phy = WgPhy::new(outbound, 1280);
        assert!(phy.receive(Instant::from_micros(0)).is_none());

        let expected = Packet::<[u8]>::from(packet(b"inbound")).as_ref().to_vec();
        phy.push_inbound(packet(b"inbound"));

        let (rx, _tx) = phy.receive(Instant::from_micros(0)).expect("a rx token");
        let seen = rx.consume(|bytes| bytes.to_vec());
        assert_eq!(seen, expected);
        assert!(
            phy.receive(Instant::from_micros(0)).is_none(),
            "the packet must be consumed exactly once"
        );
    }

    #[tokio::test]
    async fn a_full_inbound_queue_drops_the_oldest_rather_than_growing() {
        let (_to_stack, _to_device, _inbound, outbound) = ip_channels(1280);
        let mut phy = WgPhy::new(outbound, 1280);
        for _ in 0..PACKET_QUEUE_DEPTH + 4 {
            phy.push_inbound(packet(b"flood"));
        }
        assert_eq!(phy.inbound.len(), PACKET_QUEUE_DEPTH);
        assert_eq!(phy.drop_counts(), (0, 4));
    }

    #[tokio::test]
    async fn a_congested_link_refuses_a_transmit_token_instead_of_buffering() {
        let (_to_stack, _to_device, _inbound, outbound) = ip_channels(1280);
        let bytes = Packet::<[u8]>::from(packet(b"x")).as_ref().to_vec();
        let mut phy = WgPhy::new(outbound, 1280);

        // Fill the outbound channel without anyone draining it.
        for _ in 0..PACKET_QUEUE_DEPTH {
            let token = phy.transmit(Instant::from_micros(0)).expect("a tx token");
            token.consume(bytes.len(), |buffer| buffer.copy_from_slice(&bytes));
        }
        assert!(
            phy.transmit(Instant::from_micros(0)).is_none(),
            "back-pressure must reach smoltcp as a refused token"
        );
        assert_eq!(
            phy.drop_counts(),
            (0, 0),
            "nothing was dropped, only deferred"
        );
    }
}
