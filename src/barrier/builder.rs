use super::{Barrier, Waiter};
use crate::util::codec::EmptyCodec;
use crate::util::{accept::Accept, listener::WriteListener, split::Split};
use crate::{channel, multi_channel};
use errors::*;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

pub(crate) fn new_waiter<A: 'static + Clone + Send + ToSocketAddrs>(
    dest: A,
) -> WaiterBuilderFuture<
    A,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
> {
    WaiterBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn new_barrier<A: 'static + Send + Clone + ToSocketAddrs>(
    local_addr: A,
) -> BarrierBuilderFuture<
    A,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec>>>,
> {
    BarrierBuilderFuture {
        fut: multi_channel::builder::new_multi(local_addr),
    }
}

#[derive(Debug)]
#[pin_project]
pub struct WaiterBuilderFuture<A, Filter, Fut, S = TcpStream> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, (), EmptyCodec, Filter, Fut, S>,
}

impl<A, Filter, Fut> WaiterBuilderFuture<A, Filter, Fut>
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
    > {
        WaiterBuilderFuture {
            fut: self.fut.retry(retry_sleep_duration, max_retries),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> WaiterBuilderFuture<
        A,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
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
        self,
        reuseport: bool,
    ) -> WaiterBuilderFuture<
        A,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> WaiterBuilderFuture<
        A,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> WaiterBuilderFuture<
        A,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> WaiterBuilderFuture<
        A,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> WaiterBuilderFuture<
        A,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> WaiterBuilderFuture<
        A,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
    > {
        WaiterBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, Fut, Filter, S> Future for WaiterBuilderFuture<A, Filter, Fut, S>
where
    Fut: Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec, S>>>,
    S: Split,
{
    type Output = Result<Waiter<S::Left>, WaiterBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(WaiterBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Waiter(channel.split().0.into())))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct BarrierBuilderFuture<A, Filter, Fut, const N: usize = 0, L = TcpListener>
where
    Filter: Fn(SocketAddr) -> bool,
    Fut: Future<
        Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N, L>>,
    >,
    L: Accept,
{
    #[pin]
    fut: multi_channel::builder::ChannelBuilderFuture<A, (), EmptyCodec, Filter, Fut, N, L>,
}

impl<A, Filter, Fut, const N: usize> BarrierBuilderFuture<A, Filter, Fut, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
{
    pub fn limit(
        self,
        limit: usize,
    ) -> BarrierBuilderFuture<
        A,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.limit(limit),
        }
    }

    pub fn accept(
        self,
        accept: usize,
    ) -> BarrierBuilderFuture<
        A,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter2,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, M>>>,
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
        Filter2,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, M>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, M>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, Filter, Fut, const N: usize, L> Future for BarrierBuilderFuture<A, Filter, Fut, N, L>
where
    Fut: Future<
        Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N, L>>,
    >,
    Filter: Fn(SocketAddr) -> bool,
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Output = Result<Barrier<N, WriteListener<L>>, BarrierBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(BarrierBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Barrier(channel.split().1.into())))
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(display("[BarrierBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct BarrierBuilderError {
        /// source Error
        source: multi_channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[WaiterBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct WaiterBuilderError {
        /// source Error
        source: channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }
}
