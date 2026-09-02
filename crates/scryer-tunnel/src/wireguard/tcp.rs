//! Tokio-shaped TCP over the tunnel's smoltcp stack.
//!
//! [`WgTcpStream`] is an ordinary [`AsyncRead`] + [`AsyncWrite`], which is all
//! [`crate::TunnelStream`] asks for, so a WireGuard dial drops into the SOCKS5
//! front and every consumer above it without any of them learning a new type.
//!
//! Every operation follows the same three steps: take the stack lock, try the
//! smoltcp call, and on would-block register *this task's* waker on the socket
//! before returning `Pending`. smoltcp wakes those wakers from the stack pump,
//! so a blocked reader costs nothing until its bytes arrive.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp;
use smoltcp::wire::IpEndpoint;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::error::TunnelError;
use crate::wireguard::stack::StackShared;

/// A TCP connection to something on the far side of the tunnel.
pub struct WgTcpStream {
    shared: Arc<StackShared>,
    handle: SocketHandle,
    peer: SocketAddr,
    /// Set once `poll_shutdown` (or a fatal error) has closed our half, so a
    /// second shutdown is a no-op rather than a state-machine violation.
    shut_down: bool,
    /// Whether this stream holds one of the stack's socket slots. Dialled
    /// streams do; a stream handed over by [`WgTcpListener`] does not, because
    /// the listener's fixed backlog is what bounds those.
    counted: bool,
}

impl WgTcpStream {
    /// Dial `remote` and wait for the connection to establish.
    ///
    /// The caller has already claimed a socket slot; on any failure the socket
    /// is removed immediately rather than retired, because a connection that
    /// never opened has nothing to flush.
    pub(crate) async fn connect(
        shared: Arc<StackShared>,
        remote: SocketAddr,
        timeout: Duration,
    ) -> Result<WgTcpStream, TunnelError> {
        let endpoint = IpEndpoint::from(remote);
        let (handle, started) = shared.with_stack(|stack| {
            let handle = stack.add_tcp_socket();
            let local_port = stack.next_ephemeral_port();
            let (sockets, context) = stack.socket_and_context();
            let started = sockets
                .get_mut::<tcp::Socket>(handle)
                .connect(context, endpoint, local_port);
            (handle, started)
        });

        if let Err(error) = started {
            shared.with_stack(|stack| stack.discard(handle));
            return Err(TunnelError::Dial {
                host: remote.ip().to_string(),
                port: remote.port(),
                detail: match error {
                    tcp::ConnectError::Unaddressable => {
                        "the tunnel has no address in that destination's family, so it cannot \
                         reach it"
                            .to_string()
                    }
                    tcp::ConnectError::InvalidState => {
                        "the tunnel could not open a socket for that destination".to_string()
                    }
                },
            });
        }

        let established = tokio::time::timeout(
            timeout,
            std::future::poll_fn(|context| {
                shared.peek_stack(|stack| {
                    let socket = stack.sockets_mut().get_mut::<tcp::Socket>(handle);
                    match socket.state() {
                        // Still shaking hands.
                        tcp::State::SynSent | tcp::State::SynReceived => {
                            socket.register_send_waker(context.waker());
                            Poll::Pending
                        }
                        // Refused, or reset before it ever opened.
                        tcp::State::Closed | tcp::State::TimeWait => Poll::Ready(false),
                        _ => Poll::Ready(true),
                    }
                })
            }),
        )
        .await;

        match established {
            Ok(true) => Ok(WgTcpStream {
                shared,
                handle,
                peer: remote,
                shut_down: false,
                counted: true,
            }),
            Ok(false) => {
                shared.with_stack(|stack| stack.discard(handle));
                Err(TunnelError::Dial {
                    host: remote.ip().to_string(),
                    port: remote.port(),
                    detail: "the far side refused the connection".to_string(),
                })
            }
            Err(_) => {
                shared.with_stack(|stack| {
                    stack.sockets_mut().get_mut::<tcp::Socket>(handle).abort();
                    stack.discard(handle);
                });
                Err(TunnelError::Dial {
                    host: remote.ip().to_string(),
                    port: remote.port(),
                    detail: "the tunnel did not open a connection in time".to_string(),
                })
            }
        }
    }

    /// Adopt a socket that a listener has already seen become established.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn from_accepted(
        shared: Arc<StackShared>,
        handle: SocketHandle,
        peer: SocketAddr,
    ) -> Self {
        Self {
            shared,
            handle,
            peer,
            shut_down: false,
            counted: false,
        }
    }

    /// The far-side address this stream is connected to.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    fn close_half(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        let handle = self.handle;
        self.shared.with_stack(|stack| {
            stack.sockets_mut().get_mut::<tcp::Socket>(handle).close();
        });
    }
}

impl Drop for WgTcpStream {
    fn drop(&mut self) {
        // Close rather than abort: a consumer that drops a stream after a
        // complete HTTP response should still send its FIN, or the far side
        // logs a reset for every request. The pump reclaims the socket once
        // the close finishes, or gives up on it after the linger window.
        self.close_half();
        let handle = self.handle;
        self.shared.with_stack(|stack| stack.retire(handle));
        if self.counted {
            self.shared.release_socket();
        }
    }
}

impl AsyncRead for WgTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let handle = this.handle;
        let read = this.shared.peek_stack(|stack| {
            let socket = stack.sockets_mut().get_mut::<tcp::Socket>(handle);
            if !socket.may_recv() {
                // Either the far side sent its FIN or the connection is gone.
                // Both are end-of-stream to a reader; a reset that lost data
                // is reported by the writer, which is where it matters.
                return Poll::Ready(Ok(0));
            }
            match socket.recv_slice(buffer.initialize_unfilled()) {
                Ok(0) => {
                    socket.register_recv_waker(context.waker());
                    Poll::Pending
                }
                Ok(read) => Poll::Ready(Ok(read)),
                // `Finished` is a clean end of stream; `InvalidState` means the
                // socket is not connected, which is the same to a reader.
                Err(tcp::RecvError::Finished | tcp::RecvError::InvalidState) => Poll::Ready(Ok(0)),
            }
        });

        match read {
            Poll::Ready(Ok(read)) => {
                if read > 0 {
                    buffer.advance(read);
                    // Consuming bytes opened the receive window; the peer only
                    // learns that when the stack next polls.
                    this.shared.wake();
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for WgTcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.shut_down {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "this tunnel connection has been shut down",
            )));
        }
        let handle = this.handle;
        let written = this.shared.peek_stack(|stack| {
            let socket = stack.sockets_mut().get_mut::<tcp::Socket>(handle);
            if !socket.may_send() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the far side closed this tunnel connection",
                )));
            }
            match socket.send_slice(data) {
                Ok(0) => {
                    socket.register_send_waker(context.waker());
                    Poll::Pending
                }
                Ok(written) => Poll::Ready(Ok(written)),
                Err(tcp::SendError::InvalidState) => Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the far side closed this tunnel connection",
                ))),
            }
        });
        if matches!(written, Poll::Ready(Ok(_))) {
            // The bytes are in the send buffer; only a poll turns them into
            // segments on the wire.
            this.shared.wake();
        }
        written
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let handle = this.handle;
        this.shared.peek_stack(|stack| {
            let socket = stack.sockets_mut().get_mut::<tcp::Socket>(handle);
            if socket.send_queue() == 0 {
                return Poll::Ready(Ok(()));
            }
            if !socket.may_send() && socket.state() == tcp::State::Closed {
                // Nothing will ever drain: the connection is gone.
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the tunnel connection closed before everything was sent",
                )));
            }
            socket.register_send_waker(context.waker());
            Poll::Pending
        })
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        // A half-close: our FIN goes out, the far side may keep sending, and
        // reads keep working. That is what `AsyncWrite::shutdown` means, and
        // it is what a SOCKS5 relay needs when one direction finishes first.
        let this = self.get_mut();
        this.close_half();
        Poll::Ready(Ok(()))
    }
}

/// Accepts TCP connections arriving through the tunnel.
///
/// Only the test peer uses this: production tunnels are dialled out of, never
/// into. smoltcp has no listen backlog — a listening socket *becomes* the
/// connection — so a backlog here is a small pool of sockets all listening on
/// the same port, replenished as each one is taken.
#[cfg(any(test, feature = "test-support"))]
pub(crate) struct WgTcpListener {
    shared: Arc<StackShared>,
    port: u16,
    listening: Vec<SocketHandle>,
}

#[cfg(any(test, feature = "test-support"))]
impl WgTcpListener {
    pub(crate) fn bind(
        shared: Arc<StackShared>,
        port: u16,
        backlog: usize,
    ) -> Result<Self, TunnelError> {
        let mut listening = Vec::with_capacity(backlog);
        for _ in 0..backlog.max(1) {
            listening.push(listen_one(&shared, port)?);
        }
        Ok(Self {
            shared,
            port,
            listening,
        })
    }

    /// Wait for the next connection.
    pub(crate) async fn accept(&mut self) -> Result<WgTcpStream, TunnelError> {
        loop {
            let ready = std::future::poll_fn(|context| {
                self.shared.peek_stack(|stack| {
                    for (index, handle) in self.listening.iter().enumerate() {
                        let socket = stack.sockets_mut().get_mut::<tcp::Socket>(*handle);
                        // `is_active` would also report a half-open
                        // `SynReceived`, and handing that to a reader would
                        // look like an immediate end of stream. Wait for the
                        // handshake to finish.
                        if socket.may_send() || socket.may_recv() {
                            let peer = socket.remote_endpoint();
                            return Poll::Ready(Some((index, peer)));
                        }
                        socket.register_recv_waker(context.waker());
                        socket.register_send_waker(context.waker());
                    }
                    Poll::Pending
                })
            })
            .await;

            let Some((index, peer)) = ready else {
                continue;
            };
            let handle = self.listening.swap_remove(index);
            self.listening.push(listen_one(&self.shared, self.port)?);

            let peer = peer
                .map(|endpoint| SocketAddr::new(endpoint.addr.into(), endpoint.port))
                .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));
            return Ok(WgTcpStream::from_accepted(
                Arc::clone(&self.shared),
                handle,
                peer,
            ));
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for WgTcpListener {
    fn drop(&mut self) {
        let handles = std::mem::take(&mut self.listening);
        self.shared.with_stack(|stack| {
            for handle in handles {
                stack.sockets_mut().get_mut::<tcp::Socket>(handle).abort();
                stack.discard(handle);
            }
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
fn listen_one(shared: &Arc<StackShared>, port: u16) -> Result<SocketHandle, TunnelError> {
    shared.with_stack(|stack| {
        let handle = stack.add_tcp_socket();
        match stack
            .sockets_mut()
            .get_mut::<tcp::Socket>(handle)
            .listen(port)
        {
            Ok(()) => Ok(handle),
            Err(error) => {
                stack.discard(handle);
                Err(TunnelError::Engine(format!(
                    "the tunnel could not listen on port {port}: {error:?}"
                )))
            }
        }
    })
}
