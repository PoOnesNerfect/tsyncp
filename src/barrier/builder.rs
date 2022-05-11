use super::{Barrier, Waiter};
use crate::util::codec::EmptyCodec;
use crate::util::{listener::WriteListener, Accept, Split};
use crate::{channel, multi_channel};
use futures::{ready, Future};
use pin_project::pin_project;
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
    fut: channel::builder::BuilderFuture<A, (), EmptyCodec, Filter, Fut, S>,
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
    type Output = Result<Waiter<S::Left>, channel::builder::errors::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Waiter(channel.split().0.into())))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct BarrierBuilderFuture<A, Fut, const N: usize = 0, L = TcpListener>
where
    Fut: Future<
        Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N, L>>,
    >,
    L: Accept,
{
    #[pin]
    fut: multi_channel::builder::BuilderFuture<A, (), EmptyCodec, Fut, N, L>,
}

impl<A, Fut, const N: usize, L> BarrierBuilderFuture<A, Fut, N, L>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
    Fut: Future<
        Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N, L>>,
    >,
{
    /// Before returning a [Barrier], first accept the given number of connections.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut barrier: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .accept()               // accept connections before returning. (default: 1)
    ///         .num(10)                // accept 10 connections.
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn accept(
        self,
    ) -> multi_channel::builder::AcceptBuilderFuture<
        Self,
        Barrier<N, WriteListener<L>>,
        (),
        EmptyCodec,
        N,
        WriteListener<L>,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    > {
        multi_channel::builder::AcceptBuilderFuture::new(self, |_| {}, |_| true)
    }
}

impl<A, Fut, const N: usize> BarrierBuilderFuture<A, Fut, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
{
    pub fn limit(
        self,
        limit: usize,
    ) -> BarrierBuilderFuture<
        A,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.limit(limit),
        }
    }

    pub fn limit_const<const M: usize>(
        self,
    ) -> BarrierBuilderFuture<
        A,
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
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N>>>,
        N,
    > {
        BarrierBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, Fut, const N: usize, L> Future for BarrierBuilderFuture<A, Fut, N, L>
where
    Fut: Future<
        Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec, N, L>>,
    >,
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Output =
        Result<Barrier<N, WriteListener<L>>, multi_channel::builder::errors::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Barrier(channel.split().1.into())))
    }
}
