//! Contains [SenderBuilderFuture] which builds [mpsc::Sender](super::Sender),
//! and [ReceiverBuilderFuture] which builds [mpsc::Receiver](super::Receiver) when `.await`ed.
//!
//! [SenderBuilderFuture] is returned by [sender_on](super::sender_to) function without awaiting it.
//! [ReceiverBuilderFuture] is returned by [receiver_on](super::receiver_on) function without awaiting it.
//!
//! Before awaiting the future, you can chain other methods on it to configure Sender and Receiver.
//!
//! To see all available configurations, see [SenderBuilderFuture] and [ReceiverBuilderFuture].

use super::{Receiver, Sender};
use crate::util::{listener::ReadListener, Accept, Split};
use crate::{channel, multi_channel};
use futures::{ready, Future};
use pin_project::pin_project;
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
    fut: channel::builder::BuilderFuture<A, T, E, Filter, Fut, S>,
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
    type Output = Result<Sender<T, E, S::Right>, channel::builder::errors::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(channel.split().1))
    }
}

/// Future used to configure and build [Receiver](super::Receiver).
///
/// Use [receiver_on](super::receiver_on) function to create the [ReceiverBuilderFuture].
///
/// You can chain any number of configurations to the future:
///
/// ```no_run
/// use tsyncp::mpsc;
/// use serde::{Serialize, Deserialize};
/// use std::time::Duration;
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct Dummy;
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut ch: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
///         .limit(20)              // limit the total number of possible connections to 20.
///         .accept(10)             // accept 10 connections before returning.
///         .set_tcp_linger(Some(Duration::from_millis(10_000)))
///         .set_tcp_ttl(60_000)
///         .set_tcp_nodelay(true)
///         .set_tcp_reuseaddr(true)
///         .set_tcp_reuseport(true)
///         .set_tcp_send_buffer_size(8 * 1024 * 1024)
///         .set_tcp_recv_buffer_size(8 * 1024 * 1024)
///         .await?;
///
///     Ok(())
/// }
/// ```
///
/// However, there are some exclusive futures:
/// - You can only use one of [ReceiverBuilderFuture::accept], [ReceiverBuilderFuture::accept_to_limit], [ReceiverBuilderFuture::accept_filter],
/// and [ReceiverBuilderFuture::accept_to_limit_filter].
/// - You can only use one of [ReceiverBuilderFuture::limit] and [ReceiverBuilderFuture::limit_const].
#[derive(Debug)]
#[pin_project]
pub struct ReceiverBuilderFuture<A, T, E, Fut, const N: usize = 0, L = TcpListener>
where
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N, L>>>,
    L: Accept,
{
    #[pin]
    fut: multi_channel::builder::BuilderFuture<A, T, E, Fut, N, L>,
}

impl<A, T, E, Fut, const N: usize, L> ReceiverBuilderFuture<A, T, E, Fut, N, L>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N, L>>>,
{
    /// Before returning a [Sender], first accept a connection, or a given number of connections.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::broadcast;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000")
    ///         .accept()
    ///         .num(10)             // accept 10 connections before returning.
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn accept(
        self,
    ) -> multi_channel::builder::AcceptBuilderFuture<
        Self,
        Receiver<T, E, N, ReadListener<L>>,
        T,
        E,
        N,
        ReadListener<L>,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    > {
        multi_channel::builder::AcceptBuilderFuture::new(self, |_| {}, |_| true)
    }
}
impl<A, T, E, Fut, const N: usize> ReceiverBuilderFuture<A, T, E, Fut, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
{
    /// Limit the total number of connections this receiver can have.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
    ///         .limit(10)                              // limit the total number of possible connections to 10.
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn limit(
        self,
        limit: usize,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.limit(limit),
        }
    }

    /// Limit the total number of connections this receiver can have using const generic usize value.
    ///
    /// Use this method if you want to use an array instead of a vec for the [connection pool](crate::util::stream_pool::StreamPool)
    /// that handles all the connections.
    /// Using an array on the stack may improve performance by reducing access time to the streams.
    ///
    /// For more information about using an array or vec, see [StreamPool](crate::util::stream_pool::StreamPool).
    ///
    /// If you use this method, you must specify this value as the second paramter in the type
    /// specifier, as shown below.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: mpsc::JsonReceiver<Dummy, 10> = mpsc::receiver_on("localhost:8000")
    ///         .limit_const::<10>()            // ^--- this value must be set. Can be `_`.
    ///         .accept_to_limit()                  // accept up to the limit (10).
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn limit_const<const M: usize>(
        self,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, M>>>,
        M,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.limit_const(),
        }
    }

    /// Set tcp reuseaddr for all the connections made on this receiver.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
    ///         .set_tcp_reuseaddr(true)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_tcp_reuseaddr(
        self,
        reuseaddr: bool,
    ) -> ReceiverBuilderFuture<
        A,
        T,
        E,
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
        impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N>>>,
        N,
    > {
        ReceiverBuilderFuture {
            fut: self.fut.set_tcp_send_buffer_size(size),
        }
    }
}

impl<A, T, E, Fut, const N: usize, L> Future for ReceiverBuilderFuture<A, T, E, Fut, N, L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
    Fut: Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E, N, L>>>,
{
    type Output =
        Result<Receiver<T, E, N, ReadListener<L>>, multi_channel::builder::errors::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(channel.split().0))
    }
}
