use crate::{
    channel, multi_channel,
    util::{codec::EmptyCodec, split::TcpSplit},
};
use errors::*;
use futures::{ready, Future, Sink, SinkExt, Stream, StreamExt};
use snafu::{Backtrace, ResultExt};
use std::{
    fmt,
    net::{SocketAddr, ToSocketAddrs},
    task::Poll,
};
use tokio::io::{AsyncRead, AsyncWrite};

pub mod builder;

pub fn block_on<A: 'static + Clone + Send + ToSocketAddrs>(
    local_addr: A,
) -> builder::BarrierBuilderFuture<
    A,
    TcpSplit,
    impl Future<Output = multi_channel::builder::AcceptResult>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::new_barrier(local_addr)
}

pub fn wait_to<A: 'static + Clone + Send + ToSocketAddrs>(
    dest: A,
) -> builder::WaiterBuilderFuture<
    A,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::new_waiter(dest)
}

#[pin_project::pin_project]
pub struct Waiter<RW = TcpSplit>(#[pin] pub(crate) channel::Channel<(), EmptyCodec, RW>);

impl<RW> Waiter<RW> {
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<RW: AsyncRead + Unpin> Waiter<RW> {
    pub async fn wait(&mut self) -> Option<Result<(), WaiterError>> {
        self.next().await
    }
}

impl<RW: AsyncRead> Stream for Waiter<RW> {
    type Item = Result<(), WaiterError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match ready!(self.project().0.poll_next(cx)) {
            Some(Ok(item)) => Poll::Ready(Some(Ok(item))),
            Some(Err(e)) => Poll::Ready(Some(Err(e).context(WaiterSnafu))),
            None => Poll::Ready(None),
        }
    }
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Barrier<const N: usize = 0, RW = TcpSplit>(
    #[pin] multi_channel::Channel<(), EmptyCodec, N, RW>,
);

impl<const N: usize, RW> Barrier<N, RW> {
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

impl<const N: usize> Barrier<N> {
    pub async fn accept(&mut self) -> Result<SocketAddr, BarrierAcceptingError<TcpSplit>> {
        self.0.accept().await.context(BarrierAcceptingSnafu)
    }
}

impl<const N: usize, RW: 'static + fmt::Debug + AsyncWrite + Unpin> Barrier<N, RW> {
    pub async fn release(&mut self) -> Result<(), BarrierError> {
        SinkExt::send(self, ()).await
    }

    pub async fn release_to(&mut self, addrs: &[SocketAddr]) -> Result<(), BarrierError> {
        self.0.send_to((), addrs).await.context(BarrierSnafu)
    }

    pub async fn release_filtered<Filter: Fn(&SocketAddr) -> bool>(
        &mut self,
        filter: Filter,
    ) -> Result<(), BarrierError> {
        self.0.send_filtered((), filter).await.context(BarrierSnafu)
    }
}

impl<const N: usize, RW: 'static + fmt::Debug + AsyncWrite + Unpin> Sink<()> for Barrier<N, RW> {
    type Error = BarrierError;

    fn start_send(self: std::pin::Pin<&mut Self>, item: ()) -> Result<(), Self::Error> {
        self.project().0.start_send(item).context(BarrierSnafu)
    }

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_ready(cx)) {
            return Poll::Ready(Err(e).context(BarrierSnafu));
        }

        Poll::Ready(Ok(()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_flush(cx)) {
            return Poll::Ready(Err(e).context(BarrierSnafu));
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_close(cx)) {
            return Poll::Ready(Err(e).context(BarrierSnafu));
        }

        Poll::Ready(Ok(()))
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(display("[BarrierAcceptingError] Failed to accept stream"))]
    #[snafu(visibility(pub(super)))]
    pub struct BarrierAcceptingError<T: 'static + fmt::Debug> {
        source: multi_channel::errors::AcceptingError<T>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[BarrierError] Failed to send item on broadcast::Waiter"))]
    #[snafu(visibility(pub(super)))]
    pub struct BarrierError {
        source: multi_channel::errors::ChannelSinkError<(), EmptyCodec>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[WaiterError] Failed to receiver item on broadcast::Barrier"))]
    #[snafu(visibility(pub(super)))]
    pub struct WaiterError {
        source: channel::errors::ChannelStreamError<(), EmptyCodec>,
        backtrace: Backtrace,
    }
}
