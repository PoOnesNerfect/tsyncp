use super::{Receiver, Sender};
use crate::channel;
use crate::util::Split;
use errors::*;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::net::TcpStream;

pub(crate) fn sender_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> SenderToBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    SenderToBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn sender_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> SenderOnBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    SenderOnBuilderFuture {
        fut: channel::builder::new(local_addr, true),
    }
}

pub(crate) fn receiver_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> ReceiverToBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    ReceiverToBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn receiver_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> ReceiverOnBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    ReceiverOnBuilderFuture {
        fut: channel::builder::new(local_addr, true),
    }
}

#[derive(Debug)]
#[pin_project]
pub struct SenderToBuilderFuture<A, T, E, Filter, Fut, S = TcpStream> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, S>,
}

impl<A, T, E, Filter, Fut> SenderToBuilderFuture<A, T, E, Filter, Fut>
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        SenderToBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Filter, Fut, S> Future for SenderToBuilderFuture<A, T, E, Filter, Fut, S>
where
    S: Split,
    Fut: Future<Output = channel::builder::Result<channel::Channel<T, E, S>>>,
{
    type Output = Result<Sender<T, E, S::Right>, SenderBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(SenderBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Sender(channel.split().1.into())))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct SenderOnBuilderFuture<A, T, E, Filter, Fut, S = TcpStream> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, S>,
}

impl<A, T, E, Filter, Fut> SenderOnBuilderFuture<A, T, E, Filter, Fut>
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
        Filter2,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        SenderOnBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Filter, Fut, S> Future for SenderOnBuilderFuture<A, T, E, Filter, Fut, S>
where
    S: Split,
    Fut: Future<Output = channel::builder::Result<channel::Channel<T, E, S>>>,
{
    type Output = Result<Sender<T, E, S::Right>, SenderBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(SenderBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Sender(channel.split().1.into())))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct ReceiverToBuilderFuture<A, T, E, Filter, Fut, S = TcpStream> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, S>,
}

impl<A, T, E, Filter, Fut> ReceiverToBuilderFuture<A, T, E, Filter, Fut>
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        ReceiverToBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Fut, Filter, S> Future for ReceiverToBuilderFuture<A, T, E, Filter, Fut, S>
where
    S: Split,
    Fut: Future<Output = channel::builder::Result<channel::Channel<T, E, S>>>,
{
    type Output = Result<Receiver<T, E, S::Left>, ReceiverBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(ReceiverBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Receiver(channel.split().0.into())))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct ReceiverOnBuilderFuture<A, T, E, Filter, Fut, S = TcpStream> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, S>,
}

impl<A, T, E, Filter, Fut> ReceiverOnBuilderFuture<A, T, E, Filter, Fut>
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
        Filter2,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        ReceiverOnBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Filter, Fut, S> Future for ReceiverOnBuilderFuture<A, T, E, Filter, Fut, S>
where
    S: Split,
    Fut: Future<Output = channel::builder::Result<channel::Channel<T, E, S>>>,
{
    type Output = Result<Receiver<T, E, S::Left>, ReceiverBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(ReceiverBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Receiver(channel.split().0.into())))
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
        source: channel::builder::errors::ChannelBuilderError,
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
