use super::{Receiver, Sender};
use crate::util::split::TcpSplit;
use crate::{channel, multi_channel};
use errors::*;
use futures::future::Ready;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) fn new_receiver<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> ReceiverBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    ReceiverBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn new_sender<A: 'static + Send + Clone + ToSocketAddrs, T, E>(
    local_addr: A,
) -> SenderBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = multi_channel::builder::AcceptResult>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    SenderBuilderFuture {
        fut: multi_channel::builder::new_multi(local_addr),
    }
}

#[pin_project]
pub struct ReceiverBuilderFuture<A, T, E, RW, Fut, Filter> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, RW, Fut, Filter>,
}

impl<A, T, E, RW, Fut, Filter> ReceiverBuilderFuture<A, T, E, RW, Fut, Filter>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn retry(
        self,
        retry_sleep_duration: Duration,
        max_retries: usize,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.retry(retry_sleep_duration, max_retries),
        }
    }

    pub fn set_tcp_reuseaddr(
        mut self,
        reuseaddr: bool,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
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
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        mut self,
        dur: Option<Duration>,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        mut self,
        nodelay: bool,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        mut self,
        ttl: u32,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        mut self,
        size: u32,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        mut self,
        size: u32,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, RW, Fut, Filter> ReceiverBuilderFuture<A, T, E, RW, Fut, Filter> {
    pub fn with_codec<C>(self) -> ReceiverBuilderFuture<A, T, C, RW, Fut, Filter> {
        ReceiverBuilderFuture {
            fut: self.fut.with_codec(),
        }
    }

    pub fn with_stream<S>(
        self,
        custom_stream: S,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        S,
        Ready<Result<S, channel::builder::errors::ChannelBuilderError>>,
        Filter,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.with_stream(custom_stream, local_addr, peer_addr),
        }
    }
}

impl<
        A,
        T,
        E,
        RW: AsyncRead + AsyncWrite + std::fmt::Debug,
        Fut: Future<Output = channel::builder::BuildResult<RW>>,
        Filter,
    > Future for ReceiverBuilderFuture<A, T, E, RW, Fut, Filter>
where
    T: fmt::Debug,
{
    type Output = Result<Receiver<T, E, RW>, ReceiverBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(ReceiverBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, ReceiverBuilderError>(Receiver(channel)))
    }
}

#[pin_project]
pub struct SenderBuilderFuture<
    A,
    T,
    E,
    RW,
    Fut: Future<Output = multi_channel::builder::AcceptResult<N, RW>>,
    Filter: Fn(SocketAddr) -> bool,
    const N: usize = 0,
> {
    #[pin]
    fut: multi_channel::builder::ChannelBuilderFuture<A, T, E, RW, Fut, Filter, N>,
}

impl<
        A,
        T,
        E,
        RW,
        Fut: Future<Output = multi_channel::builder::AcceptResult<N, RW>>,
        Filter: Fn(SocketAddr) -> bool,
        const N: usize,
    > SenderBuilderFuture<A, T, E, RW, Fut, Filter, N>
{
    pub fn limit(self, limit: usize) -> Self {
        SenderBuilderFuture {
            fut: self.fut.limit(limit),
        }
    }

    pub fn with_codec<C>(self) -> SenderBuilderFuture<A, T, C, RW, Fut, Filter, N> {
        SenderBuilderFuture {
            fut: self.fut.with_codec(),
        }
    }
}

impl<
        A,
        T,
        E,
        Fut: Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter: Clone + Fn(SocketAddr) -> bool,
        const N: usize,
    > SenderBuilderFuture<A, T, E, TcpSplit, Fut, Filter, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
{
    pub fn accept(
        self,
        accept: usize,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.accept(accept),
        }
    }

    pub fn accept_full(
        self,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.accept_full(),
        }
    }

    pub fn accept_filtered<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        accept: usize,
        filter: Filter2,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<M>>,
        Filter2,
        M,
    > {
        SenderBuilderFuture {
            fut: self.fut.accept_filtered(accept, filter),
        }
    }

    pub fn accept_filtered_full<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        filter: Filter2,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<M>>,
        Filter2,
        M,
    > {
        SenderBuilderFuture {
            fut: self.fut.accept_filtered_full(filter),
        }
    }

    pub fn limit_const<const M: usize>(
        self,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<M>>,
        Filter,
        M,
    > {
        SenderBuilderFuture {
            fut: self.fut.limit_const(),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
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
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = multi_channel::builder::AcceptResult<N>>,
        Filter,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<
        A,
        T,
        E,
        RW,
        Fut: Future<Output = multi_channel::builder::AcceptResult<N, RW>>,
        Filter: Fn(SocketAddr) -> bool,
        const N: usize,
    > Future for SenderBuilderFuture<A, T, E, RW, Fut, Filter, N>
{
    type Output = Result<Sender<T, E, N, RW>, SenderBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(SenderBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, SenderBuilderError>(Sender(channel)))
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(display("[SenderBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderBuilderError {
        /// source Error
        source: multi_channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[ReceiverBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverBuilderError {
        /// source Error
        source: channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }
}
