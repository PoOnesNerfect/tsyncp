use super::{Receiver, Sender};
use crate::util::{accept::Accept, listener::WriteListener, split::Split};
use crate::{channel, multi_channel};
use futures::{ready, Future};
use pin_project::pin_project;
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

pub(crate) fn new_receiver<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> ReceiverBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E>>>,
> {
    SenderBuilderFuture {
        fut: multi_channel::builder::new_multi(local_addr),
    }
}

#[derive(Debug)]
#[pin_project]
pub struct ReceiverBuilderFuture<A, T, E, Filter, Fut, S = TcpStream> {
    #[pin]
    fut: channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, S>,
}

impl<A, T, E, Filter, Fut> ReceiverBuilderFuture<A, T, E, Filter, Fut>
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
        Filter,
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.retry(retry_sleep_duration, max_retries),
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
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
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
        impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Filter, Fut, S> Future for ReceiverBuilderFuture<A, T, E, Filter, Fut, S>
where
    Fut: Future<Output = channel::builder::Result<channel::Channel<T, E, S>>>,
    S: Split,
{
    type Output = Result<Receiver<T, E, S::Left>, channel::builder::errors::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(channel.split().0.into()))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct SenderBuilderFuture<A, T, E, Filter, Fut, const N: usize = 0, L = TcpListener>
where
    Filter: Fn(SocketAddr) -> bool,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N, L>>>,
    L: Accept,
{
    #[pin]
    fut: multi_channel::builder::ChannelBuilderFuture<A, T, E, Filter, Fut, N, L>,
}

impl<
        A,
        T,
        E,
        Filter: Clone + Fn(SocketAddr) -> bool,
        Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        const N: usize,
    > SenderBuilderFuture<A, T, E, Filter, Fut, N>
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter2,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, M>>>,
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
        Filter2,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, M>>>,
        M,
    > {
        SenderBuilderFuture {
            fut: self.fut.accept_filtered_full(filter),
        }
    }

    pub fn limit(
        self,
        limit: usize,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.limit(limit),
        }
    }

    pub fn limit_const<const M: usize>(
        self,
    ) -> SenderBuilderFuture<
        A,
        T,
        E,
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, M>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
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
        Filter,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        SenderBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Filter, Fut, const N: usize, L> Future
    for SenderBuilderFuture<A, T, E, Filter, Fut, N, L>
where
    Filter: Fn(SocketAddr) -> bool,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N, L>>>,
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Output =
        Result<Sender<T, E, N, WriteListener<L>>, multi_channel::builder::errors::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(channel.split().1))
    }
}
