//! Tokio-shaped UDP over the tunnel's smoltcp stack.
//!
//! Datagrams matter for two reasons. The obvious one is that DNS is UDP, and
//! DNS has to traverse the tunnel or the whole exercise leaks names to the
//! local network. The less obvious one is that the tunnel's *own* resolver
//! socket is not this type — smoltcp's DNS socket owns that — so this exists
//! for callers with their own datagram protocol, and for the test peer, which
//! answers DNS with it.
//!
//! There is deliberately no UDP path through the SOCKS5 front: that front is
//! CONNECT-only, and UDP ASSOCIATE stays unimplemented.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::Poll;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::udp;
use smoltcp::wire::IpEndpoint;

use crate::error::TunnelError;
use crate::wireguard::stack::StackShared;

/// A UDP socket bound inside the tunnel.
pub struct WgUdpSocket {
    shared: Arc<StackShared>,
    handle: SocketHandle,
    local_port: u16,
}

impl WgUdpSocket {
    /// Bind `port`, or an ephemeral port when it is `0`.
    pub(crate) fn bind(shared: Arc<StackShared>, port: u16) -> Result<Self, TunnelError> {
        let (handle, local_port, bound) = shared.with_stack(|stack| {
            let handle = stack.add_udp_socket();
            let local_port = if port == 0 {
                stack.next_ephemeral_port()
            } else {
                port
            };
            let bound = stack
                .sockets_mut()
                .get_mut::<udp::Socket>(handle)
                .bind(local_port);
            (handle, local_port, bound)
        });

        if let Err(error) = bound {
            shared.with_stack(|stack| stack.discard(handle));
            return Err(TunnelError::Engine(format!(
                "the tunnel could not bind UDP port {local_port}: {error:?}"
            )));
        }

        Ok(Self {
            shared,
            handle,
            local_port,
        })
    }

    /// The port this socket is bound to inside the tunnel.
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Send one datagram to `remote`.
    ///
    /// Waits for room in the send buffer rather than failing, so a caller
    /// sending faster than the link drains is slowed instead of dropped.
    pub async fn send_to(&self, data: &[u8], remote: SocketAddr) -> io::Result<()> {
        let endpoint = IpEndpoint::from(remote);
        let handle = self.handle;
        let queued = std::future::poll_fn(|context| {
            self.shared.peek_stack(|stack| {
                let socket = stack.sockets_mut().get_mut::<udp::Socket>(handle);
                match socket.send_slice(data, endpoint) {
                    Ok(()) => Poll::Ready(Ok(())),
                    Err(udp::SendError::BufferFull) => {
                        socket.register_send_waker(context.waker());
                        Poll::Pending
                    }
                    Err(udp::SendError::Unaddressable) => Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "the tunnel has no address in that destination's family",
                    ))),
                }
            })
        })
        .await;
        if queued.is_ok() {
            self.shared.wake();
        }
        queued
    }

    /// Receive one datagram, returning how much was written and who sent it.
    ///
    /// A datagram larger than `data` is truncated, exactly as a real socket
    /// truncates it.
    pub async fn recv_from(&self, data: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let handle = self.handle;
        std::future::poll_fn(|context| {
            self.shared.peek_stack(|stack| {
                let socket = stack.sockets_mut().get_mut::<udp::Socket>(handle);
                match socket.recv_slice(data) {
                    Ok((read, metadata)) => Poll::Ready(Ok((
                        read,
                        SocketAddr::new(metadata.endpoint.addr.into(), metadata.endpoint.port),
                    ))),
                    Err(udp::RecvError::Exhausted) => {
                        socket.register_recv_waker(context.waker());
                        Poll::Pending
                    }
                    Err(udp::RecvError::Truncated) => Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the datagram did not fit in the supplied buffer",
                    ))),
                }
            })
        })
        .await
    }
}

impl Drop for WgUdpSocket {
    fn drop(&mut self) {
        let handle = self.handle;
        self.shared.with_stack(|stack| {
            stack.sockets_mut().get_mut::<udp::Socket>(handle).close();
            // Unlike TCP there is nothing to flush and no close to complete,
            // so the socket goes straight back.
            stack.discard(handle);
        });
        self.shared.release_socket();
    }
}
