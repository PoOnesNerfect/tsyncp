use super::{Receiver, Sender};
use crate::util::{Accept, ReadListener, Split};
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

pub(crate) fn new_sender<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> SenderBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    SenderBuilderFuture {
        fut: channel::builder::new(dest, false),
    }
}

pub(crate) fn new_receiver<A: 'static + Send + Clone + ToSocketAddrs, T, E>(
    local_addr: A,
) -> ReceiverBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E>>>,
> {
    ReceiverBuilderFuture {
        fut: multi_channel::builder::new_multi(local_addr),
    }
}

#[derive(Debug)]
#[pin_project]
pub struct SenderBuilderFuture<A, T, E, Filter, Fut, S = TcpStream> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, S>,
}

impl<A, T, E, Filter, Fut> SenderBuilderFuture<A, T, E, Filter, Fut>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn retry(
        self,
        retry_sleep_duration: Duration,
        max_retries: usize,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        SenderBuilderFuture {
            fut: self.fut.retry(retry_sleep_duration, max_retries),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Filter, Fut, S> Future for SenderBuilderFuture<A, T, E, Filter, Fut, S>
where
    Fut: Future<Output = channel::builder::Result<channel::Channel<T, E, S>>>,
    S: Split,
{
    type Output = Result<Sender<T, E, S::Right>, SenderBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(SenderBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, SenderBuilderError>(Sender(channel.split().1)))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct ReceiverBuilderFuture<A, T, E, Filter, Fut, const N: usize = 0, L = TcpListener>
where
    Filter: Fn(SocketAddr) -> bool,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N, L>>>,
    L: Accept,
{
    #[pin]
    fut: multi_channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, N, L>,
}

impl<A, T, E, Filter, Fut, const N: usize> ReceiverBuilderFuture<A, T, E, Filter, Fut, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
{
    pub fn accept(
        self,
        accept: usize,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.accept(accept),
        }
    }

    pub fn accept_full(
        self,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.accept_full(),
        }
    }

    pub fn accept_filtered<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        accept: usize,
        filter: Filter2,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter2,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, M>>>,
        M,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.accept_filtered(accept, filter),
        }
    }

    pub fn accept_filtered_full<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        filter: Filter2,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter2,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, M>>>,
        M,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.accept_filtered_full(filter),
        }
    }

    pub fn limit(
        self,
        limit: usize,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<
            Output = Result<
                multi_channel::Channel<T, E, N>,
                multi_channel::builder::errors::ChannelBuilderError,
            >,
        >,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.limit(limit),
        }
    }

    pub fn limit_const<const M: usize>(
        self,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, M>>>,
        M,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.limit_const(),
        }
    }

    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
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
        self,
        reuseport: bool,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_reuseport(reuseport),
        }
    }

    pub fn set_tcp_linger(
        self,
        dur: Option<Duration>,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_linger(dur),
        }
    }

    pub fn set_tcp_nodelay(
        self,
        nodelay: bool,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_nodelay(nodelay),
        }
    }

    pub fn set_tcp_ttl(
        self,
        ttl: u32,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_ttl(ttl),
        }
    }

    pub fn set_tcp_recv_buffer_size(
        self,
        size: u32,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_recv_buffer_size(size),
        }
    }

    pub fn set_tcp_send_buffer_size(
        self,
        size: u32,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Filter, Fut, const N: usize, L> Future
    for ReceiverBuilderFuture<A, T, E, Filter, Fut, N, L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
    Filter: Fn(SocketAddr) -> bool,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N, L>>>,
{
    type Output = Result<Receiver<T, E, N, ReadListener<L>>, ReceiverBuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)).context(ReceiverBuilderSnafu) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Receiver(channel.split().0)))
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
        source: multi_channel::builder::errors::ChannelBuilderError,
        backtrace: Backtrace,
    }
}
