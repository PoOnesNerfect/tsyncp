//! Contains [BarrierBuilderFuture] which builds [barrier::Barrier](super::Barrier),
//! and [WaiterBuilderFuture] which builds [barrier::Waiter](super::Waiter).
//!
//! [BarrierBuilderFuture] is returned by [barrier_on](super::barrier_on) function.
//!
//! [WaiterBuilderFuture] is returned by [waiter_to](super::waiter_to) function.
//!
//! Before awaiting the future, you can chain other methods on it to configure Barrier and Waiter.
//!
//! To see all available configurations, see [BarrierBuilderFuture] and [WaiterBuilderFuture].

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

/// Future returned by [waiter_to(_)](crate::barrier::waiter_to)
/// to configure and build [Waiter](super::Waiter).
///
/// You can chain any number of configurations to the future:
///
/// ```no_run
/// use tsyncp::barrier;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
///         .retry(Duration::from_millis(500), 100)
///         .set_tcp_linger(Some(Duration::from_millis(10_000)))
///         .set_tcp_ttl(60_000)
///         .set_tcp_nodelay(true)
///         .set_tcp_reuseaddr(true)
///         .set_tcp_reuseport(true)
///         .set_tcp_send_buffer_size(8 * 1024 * 1024)
///         .set_tcp_recv_buffer_size(8 * 1024 * 1024)
///         .await?;
///
///     let _ = wx.wait().await;
///
///     Ok(())
/// }
/// ```
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
    /// Retry connecting to remote [Barrier]'s address for `max_retries` with the interval
    /// `retry_sleep_duration`.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
    ///         .retry(Duration::from_millis(500), 100)
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp reuseaddr for the connection made on this waiter.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter= barrier::waiter_to("localhost:8000")
    ///         .set_tcp_reuseaddr(true)
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp reuseport for the connection made on this waiter.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
    ///         .set_tcp_reuseport(true)
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp linger for the connection made on this waiter.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
    ///         .set_tcp_linger(Some(Duration::from_millis(10_000)))
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp nodelay for the connection made on this waiter.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
    ///         .set_tcp_nodelay(true)
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp ttl for the connection made on this waiter.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
    ///         .set_tcp_ttl(60_000)
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp recv_buffer_size for the connection made on this waiter.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
    ///         .set_tcp_recv_buffer_size(8 * 1024 * 1024)
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp send_buffer_size for the connection made on this waiter.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
    ///         .set_tcp_send_buffer_size(8 * 1024 * 1024)
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
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
    type Output = Result<Waiter<S::Left>, channel::builder::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Waiter(channel.split().0.into())))
    }
}

/// Future returned by [barrier_on(_)](super::barrier_on) to configure and build [Barrier](super::Barrier).
///
/// Use [barrier_on](super::barrier_on) function to create the [BarrierBuilderFuture].
///
/// You can chain any number of configurations to the future:
///
/// ```no_run
/// use tsyncp::barrier;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
///         .limit(20)              // limit the total number of possible connections to 20.
///         .set_tcp_linger(Some(Duration::from_millis(10_000)))
///         .set_tcp_ttl(60_000)
///         .set_tcp_nodelay(true)
///         .set_tcp_reuseaddr(true)
///         .set_tcp_reuseport(true)
///         .set_tcp_send_buffer_size(8 * 1024 * 1024)
///         .set_tcp_recv_buffer_size(8 * 1024 * 1024)
///         .accept()
///         .to_limit()             // accept connections to limit before returning.
///         .await?;
///
///     bx.release().await?;
///
///     Ok(())
/// }
/// ```
///
/// However, there are some exclusive futures:
/// - You can only use one of [BarrierBuilderFuture::limit] and [BarrierBuilderFuture::limit_const].
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
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .accept()               // accept connections before returning. (default: 1)
    ///         .num(10)                // accept 10 connections.
    ///         .await?;
    ///
    ///     bx.release().await?;
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
    /// Limit the total number of waiters this barrier can have.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .limit(10)                              // limit the total number of possible connections to 10.
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Limit the total number of connections this barrier can have using const generic usize value.
    ///
    /// Use this method if you want to use an array instead of a vec for the [StreamPool](crate::util::stream_pool::StreamPool)
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
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier<10> = barrier::barrier_on("localhost:8000")
    ///         .limit_const::<10>()    // ^--- this value must be set. Can be `_`.
    ///         .accept()
    ///         .to_limit()             // accept up to the limit (10).
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp reuseaddr for all the connections made on this barrier.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .set_tcp_reuseaddr(true)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp reuseport for all the connections made on this barrier.
    ///
    /// *Warning:* only available to unix targets excluding "solaris" and "illumos".
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .set_tcp_reuseport(true)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp linger for all the connections made on this barrier.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    /// use serde::{Serialize, Deserialize};
    /// use std::time::Duration;
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .set_tcp_linger(Some(Duration::from_millis(10_000)))
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp nodelay for all the connections made on this barrier.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .set_tcp_nodelay(true)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp ttl for all the connections made on this barrier.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .set_tcp_ttl(60_000)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp recv_buffer_size for all the connections made on this barrier.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .set_tcp_recv_buffer_size(8 * 1024 * 1024)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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

    /// Set tcp send_buffer_size for all the connections made on this barrier.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .set_tcp_send_buffer_size(8 * 1024 * 1024)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
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
    type Output = Result<Barrier<N, WriteListener<L>>, multi_channel::builder::BuilderError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let channel = match ready!(this.fut.poll(cx)) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok(Barrier(channel.split().1.into())))
    }
}
