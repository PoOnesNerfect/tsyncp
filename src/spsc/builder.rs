use super::{Receiver, Sender};
use crate::channel;
use crate::util::{frame_codec::VariedLengthDelimitedCodec, split::TcpSplit, Framed};
use errors::*;
use futures::future::Ready;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt, Snafu};
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use std::{fmt, io};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpSocket;
use tokio::task::JoinError;

pub(crate) fn sender_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> SenderToBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    SenderToBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn sender_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> SenderOnBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    SenderOnBuilderFuture {
        fut: channel::builder::new(dest, true),
    }
}

pub(crate) fn receiver_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> ReceiverToBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    ReceiverToBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn receiver_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> ReceiverOnBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    ReceiverOnBuilderFuture {
        fut: channel::builder::new(dest, true),
    }
}

/// Future returned by [sender] method in which awaiting it builds the [Channel].
///
/// This future can be optionally set custom configurations by calling methods on it such as [with_tls],
/// [with_codec], [with_frame_codec] before awaiting it.
#[pin_project]
pub struct SenderToBuilderFuture<A, T, E, RW, Fut, Filter> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, RW, Fut, Filter>,
}

impl<A, T, E, RW, Fut, Filter> SenderToBuilderFuture<A, T, E, RW, Fut, Filter>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn retry(
        self,
        retry_sleep_duration: Duration,
        max_retries: usize,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
            fut: self.fut.retry(retry_sleep_duration, max_retries),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
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
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderToBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, RW, Fut, Filter> SenderToBuilderFuture<A, T, E, RW, Fut, Filter> {
    pub fn with_codec<C>(self) -> SenderToBuilderFuture<A, T, C, RW, Fut, Filter> {
        SenderToBuilderFuture {
            fut: self.fut.with_codec(),
        }
    }

    pub fn with_stream<S>(
        self,
        custom_stream: S,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> SenderToBuilderFuture<
        A,
        T,
        E,
        S,
        Ready<Result<S, channel::builder::errors::ChannelBuilderError>>,
        Filter,
    > {
        SenderToBuilderFuture {
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
    > Future for SenderToBuilderFuture<A, T, E, RW, Fut, Filter>
where
    T: fmt::Debug,
{
    type Output = Result<Sender<T, E, RW>, SenderBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(SenderBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, SenderBuilderError>(Sender(channel)))
    }
}

/// Future returned by [sender] method in which awaiting it builds the [Channel].
///
/// This future can be optionally set custom configurations by calling methods on it such as [with_tls],
/// [with_codec], [with_frame_codec] before awaiting it.
#[pin_project]
pub struct SenderOnBuilderFuture<A, T, E, RW, Fut, Filter> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, RW, Fut, Filter>,
}

impl<A, T, E, RW, Fut, Filter> SenderOnBuilderFuture<A, T, E, RW, Fut, Filter>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn filter<Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        filter: Filter2,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter2,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.filter(filter),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderOnBuilderFuture {
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
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, RW, Fut, Filter> SenderOnBuilderFuture<A, T, E, RW, Fut, Filter> {
    pub fn with_codec<C>(self) -> SenderOnBuilderFuture<A, T, C, RW, Fut, Filter> {
        SenderOnBuilderFuture {
            fut: self.fut.with_codec(),
        }
    }

    pub fn with_stream<S>(
        self,
        custom_stream: S,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> SenderOnBuilderFuture<
        A,
        T,
        E,
        S,
        Ready<Result<S, channel::builder::errors::ChannelBuilderError>>,
        Filter,
    > {
        SenderOnBuilderFuture {
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
    > Future for SenderOnBuilderFuture<A, T, E, RW, Fut, Filter>
where
    T: fmt::Debug,
{
    type Output = Result<Sender<T, E, RW>, SenderBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(SenderBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, SenderBuilderError>(Sender(channel)))
    }
}

/// Future returned by [sender] method in which awaiting it builds the [Channel].
///
/// This future can be optionally set custom configurations by calling methods on it such as [with_tls],
/// [with_codec], [with_frame_codec] before awaiting it.
#[pin_project]
pub struct ReceiverToBuilderFuture<A, T, E, RW, Fut, Filter> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, RW, Fut, Filter>,
}

impl<A, T, E, RW, Fut, Filter> ReceiverToBuilderFuture<A, T, E, RW, Fut, Filter>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn retry(
        self,
        retry_sleep_duration: Duration,
        max_retries: usize,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.retry(retry_sleep_duration, max_retries),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
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
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, RW, Fut, Filter> ReceiverToBuilderFuture<A, T, E, RW, Fut, Filter> {
    pub fn with_codec<C>(self) -> ReceiverToBuilderFuture<A, T, C, RW, Fut, Filter> {
        ReceiverToBuilderFuture {
            fut: self.fut.with_codec(),
        }
    }

    pub fn with_stream<S>(
        self,
        custom_stream: S,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> ReceiverToBuilderFuture<
        A,
        T,
        E,
        S,
        Ready<Result<S, channel::builder::errors::ChannelBuilderError>>,
        Filter,
    > {
        ReceiverToBuilderFuture {
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
    > Future for ReceiverToBuilderFuture<A, T, E, RW, Fut, Filter>
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

/// Future returned by [sender] method in which awaiting it builds the [Channel].
///
/// This future can be optionally set custom configurations by calling methods on it such as [with_tls],
/// [with_codec], [with_frame_codec] before awaiting it.
#[pin_project]
pub struct ReceiverOnBuilderFuture<A, T, E, RW, Fut, Filter> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, RW, Fut, Filter>,
}

impl<A, T, E, RW, Fut, Filter> ReceiverOnBuilderFuture<A, T, E, RW, Fut, Filter>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn filter<Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        filter: Filter2,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter2,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.filter(filter),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
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
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        TcpSplit,
        impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, RW, Fut, Filter> ReceiverOnBuilderFuture<A, T, E, RW, Fut, Filter> {
    pub fn with_codec<C>(self) -> ReceiverOnBuilderFuture<A, T, C, RW, Fut, Filter> {
        ReceiverOnBuilderFuture {
            fut: self.fut.with_codec(),
        }
    }

    pub fn with_stream<S>(
        self,
        custom_stream: S,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> ReceiverOnBuilderFuture<
        A,
        T,
        E,
        S,
        Ready<Result<S, channel::builder::errors::ChannelBuilderError>>,
        Filter,
    > {
        ReceiverOnBuilderFuture {
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
    > Future for ReceiverOnBuilderFuture<A, T, E, RW, Fut, Filter>
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

pub mod errors {
    use super::*;
    use snafu::Snafu;

    /// Codec's error type
    #[derive(Debug, Snafu)]
    #[snafu(display("[SenderBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderBuilderError {
        /// source Error
        source: channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }

    /// Codec's error type
    #[derive(Debug, Snafu)]
    #[snafu(display("[ReceiverBuilderError] Failed building sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverBuilderError {
        /// source Error
        source: channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }
}
