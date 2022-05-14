use super::TcpStreamSettings;
use futures::ready;
use std::fmt;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use tokio::net::TcpListener;
use tokio::net::TcpStream;

/// Provides abstraction over accepting new connections.
pub trait Accept {
    /// Resulting connection.
    type Output: fmt::Debug;
    /// Config that can be applied to the stream.
    ///
    /// For `TcpListener`, config is [TcpStreamSettings](crate::util::TcpStreamSettings), and sets `nodelay` and `ttl`.
    type Config: fmt::Debug + PartialEq + Clone;
    /// Error returned while accepting.
    type Error: 'static + snafu::Error;

    /// Try polling to see if there is a new incoming connection.
    ///
    /// Outputs the new connection along with the address of the remote connection.
    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>>;

    /// Handles the case where the accept future is abruptly dropped.
    ///
    /// For most cases, implementing is not necessary; hence, it's not necessary to implement it.
    ///
    /// It's used by [ReadListener](crate::util::listener::ReadListener) and [WriteListener](crate::util::listener::WriteListener).
    /// Since they share the underlying listener, it's necessary for them to wake the other
    /// listener when it's dropped.
    fn handle_abrupt_drop(&self) {}
}

impl Accept for TcpListener {
    type Output = TcpStream;
    type Config = TcpStreamSettings;
    type Error = std::io::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>> {
        let (o, a) = ready!(self.poll_accept(cx))?;

        if let Some(nodelay) = config.nodelay {
            o.set_nodelay(nodelay)?;
        }

        if let Some(ttl) = config.ttl {
            o.set_ttl(ttl)?;
        }

        Poll::Ready(Ok((o, a)))
    }
}
