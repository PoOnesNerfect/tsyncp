use crate::{
    channel,
    multi_channel::{self, send::SendFuture},
    util::{accept::Accept, codec::EmptyCodec, listener::WriteListener, tcp},
};
use futures::Future;
use snafu::Backtrace;
use std::net::{SocketAddr, ToSocketAddrs};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
};

pub mod builder;

pub fn block_on<A: 'static + Clone + Send + ToSocketAddrs>(
    local_addr: A,
) -> builder::BarrierBuilderFuture<
    A,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec>>>,
> {
    builder::new_barrier(local_addr)
}

pub fn wait_to<A: 'static + Clone + Send + ToSocketAddrs>(
    dest: A,
) -> builder::WaiterBuilderFuture<
    A,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
> {
    builder::new_waiter(dest)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Waiter<S = tcp::OwnedReadHalf>(#[pin] pub(crate) channel::Channel<(), EmptyCodec, S>);

impl<S> Waiter<S> {
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<S> Waiter<S>
where
    S: AsyncRead + Unpin,
{
    pub fn wait(&mut self) -> channel::recv::RecvFuture<'_, (), EmptyCodec, S> {
        self.0.recv()
    }
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Barrier<const N: usize = 0, L: Accept = WriteListener<TcpListener>>(
    #[pin] multi_channel::Channel<(), EmptyCodec, N, L>,
);

impl<const N: usize, L> Barrier<N, L>
where
    L: Accept,
{
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn limit(&self) -> Option<usize> {
        self.0.limit()
    }

    pub fn local_addr(&self) -> &SocketAddr {
        self.0.local_addr()
    }

    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.0.peer_addrs()
    }
}

impl<const N: usize, L: Accept> Barrier<N, L>
where
    L::Output: AsyncWrite + Unpin,
{
    pub fn release(&mut self) -> SendFuture<'_, (), EmptyCodec, N, L> {
        self.0.send(())
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;
    use std::io;

    #[derive(Debug, Snafu)]
    #[snafu(display("[BarrierError] Failed to release"))]
    #[snafu(visibility(pub(super)))]
    pub struct BarrierError {
        source: multi_channel::errors::SinkError<io::Error>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[WaiterError] Failed to wait"))]
    #[snafu(visibility(pub(super)))]
    pub struct WaiterError {
        source: channel::errors::StreamError<io::Error>,
        backtrace: Backtrace,
    }
}
