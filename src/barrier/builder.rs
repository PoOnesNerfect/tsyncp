use super::{Barrier, Waiter};
use crate::util::codec::EmptyCodec;
use crate::util::split::TcpSplit;
use crate::{channel, multi_channel};
use errors::*;
use futures::future::Ready;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) fn new_waiter<A: 'static + Clone + Send + ToSocketAddrs>(
    dest: A,
) -> WaiterBuilderFuture<
    A,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    WaiterBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn new_barrier<A: 'static + Send + Clone + ToSocketAddrs>(
    local_addr: A,
) -> BarrierBuilderFuture<
    A,
    TcpSplit,
    impl Future<Output = multi_channel::builder::AcceptResult>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    BarrierBuilderFuture {
        fut: multi_channel::builder::new_multi(local_addr),
    }
}

/// Future returned by [sender] method in which awaiting it builds the [Channel].
///
/// This future can be optionally set custom configurations by calling methods on it such as [with_tls],
/// [with_codec], [with_frame_codec] before awaiting it.
#[pin_project]
pub struct WaiterBuilderFuture<A, RW, Fut, Filter> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, (), EmptyCodec, RW, Fut, Filter>,
}

impl<A, RW, Fut, Filter> WaiterBuilderFuture<A, RW, Fut, Filter>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn retry(
        self,
        retry_sleep_duration: Duration,
        max_retries: usize,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.retry(retry_sleep_duration, max_retries),
        }
    }

    pub fn set_tcp_reuseaddr(
        mut self,
        reuseaddr: bool,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_reuseaddr(reuseaddr),
        }
    }

    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    #[cfg_attr(
        docsrs,
        doc(cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos"))))
    )]
    pub fn set_tcp_reuseport(
        mut self,
        reuseport: bool,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        mut self,
        dur: Option<Duration>,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        mut self,
        nodelay: bool,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        mut self,
        ttl: u32,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        mut self,
        size: u32,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        mut self,
        size: u32,
    ) -> WaiterBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, RW, Fut, Filter> WaiterBuilderFuture<A, RW, Fut, Filter> {
    pub fn with_stream<S>(
        self,
        custom_stream: S,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> WaiterBuilderFuture<
        A,
        S,
        Ready<Result<S, channel::builder::errors::ChannelBuilderError>>,
        Filter,
    > {
        WaiterBuilderFuture {
            fut: self.fut.with_stream(custom_stream, local_addr, peer_addr),
        }
    }
}

impl<
        A,
        RW: AsyncRead + AsyncWrite + std::fmt::Debug,
        Fut: Future<Output = channel::builder::BuildResult<RW>>,
        Filter,
    > Future for WaiterBuilderFuture<A, RW, Fut, Filter>
{
    type Output = Result<Waiter<RW>, WaiterBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(WaiterBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, WaiterBuilderError>(Waiter(channel)))
    }
}

#[pin_project]
pub struct BarrierBuilderFuture<
    A,
    RW,
    Fut: Future<Output = multi_channel::builder::AcceptResult<N, RW>>,
    Filter: Fn(SocketAddr) -> bool,
    const N: usize = 0,
> {
    #[pin]
    fut: multi_channel::builder::ChannelBuilderFuture<A, (), EmptyCodec, RW, Fut, Filter, N>,
}

impl<
        A,
        RW,
        Fut: Future<Output = multi_channel::builder::AcceptResult<N, RW>>,
        Filter: Fn(SocketAddr) -> bool,
        const N: usize,
    > BarrierBuilderFuture<A, RW, Fut, Filter, N>
{
    pub fn limit(self, limit: usize) -> Self {
        BarrierBuilderFuture {
            fut: self.fut.limit(limit),
        }
    }
}

impl<
        A,
        Fut: Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter: Clone + Fn(SocketAddr) -> bool,
        const N: usize,
    > BarrierBuilderFuture<A, TcpSplit, Fut, Filter, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
{
    pub fn accept(
        self,
        accept: usize,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.accept(accept),
        }
    }

    pub fn accept_full(
        self,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.accept_full(),
        }
    }

    pub fn accept_filtered<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        accept: usize,
        filter: Filter2,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<M>>,
        Filter2,
        M,
    > {
        BarrierBuilderFuture {
            fut: self.fut.accept_filtered(accept, filter),
        }
    }

    pub fn accept_filtered_full<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        filter: Filter2,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<M>>,
        Filter2,
        M,
    > {
        BarrierBuilderFuture {
            fut: self.fut.accept_filtered_full(filter),
        }
    }

    pub fn limit_const<const M: usize>(
        self,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<M>>,
        Filter,
        M,
    > {
        BarrierBuilderFuture {
            fut: self.fut.limit_const(),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_reuseaddr(reuseaddr),
        }
    }

    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    #[cfg_attr(
        docsrs,
        doc(cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos"))))
    )]
    pub fn set_tcp_reuseport(
        self,
        reuseport: bool,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> BarrierBuilderFuture<
        A,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<
        A,
        RW,
        Fut: Future<Output = multi_channel::builder::AcceptResult<N, RW>>,
        Filter: Fn(SocketAddr) -> bool,
        const N: usize,
    > Future for BarrierBuilderFuture<A, RW, Fut, Filter, N>
{
    type Output = Result<Barrier<N, RW>, BarrierBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(BarrierBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, BarrierBuilderError>(Barrier(channel)))
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    /// Codec's error type
    #[derive(Debug, Snafu)]
    #[snafu(display("[BarrierBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct BarrierBuilderError {
        /// source Error
        source: multi_channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }

    /// Codec's error type
    #[derive(Debug, Snafu)]
    #[snafu(display("[WaiterBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct WaiterBuilderError {
        /// source Error
        source: channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }
}
