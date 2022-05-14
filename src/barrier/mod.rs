//! Multiple waiters [wait](Waiter::wait) for a barrier to [Barrier::release].
//!
//! [Barrier] is initialized by listening on a local address with [barrier_on].
//!
//! [Waiter] is initialized by connecting to a remote address of the barrier with [waiter_to].
//!
//! [barrier_on] and [waiter_to] can take any type of parameter that implements [ToSocketAddrs](std::net::ToSocketAddrs).
//!
//! Since this module does not send or receive any data, it does not require any codec.
//!
//! [Skip to APIs](#modules)
//!
//! ## Example Contents
//! These examples will demonstrate how to initialize a Barrier and Waiter and how to chain configurations when initializing.
//! * [Example 1](#example-1): Basic initialization.
//! * [Example 2](#example-2): Chaining futures to initialization.
//!
//! To see how you can use a Barrier once initialized, see [Barrier's documentation](Barrier).
//! To see how you can use a Waiter once initialized, see [Waiter's documentation](Waiter).
//!
//! ### Example 1
//!
//! Below is the basic example of initializing, releasing with [Barrier].
//!
//! ##### Barrier:
//!
//! ```no_run
//! use tsyncp::barrier;
//!
//! #[tokio::main]
//! async fn main() -> color_eyre::Result<()> {
//!     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000").await?;
//!
//!     bx.release().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ##### Waiter:
//!
//! ```no_run
//! use tsyncp::barrier;
//! use serde::{Serialize, Deserialize};
//!
//! #[tokio::main]
//! async fn main() -> color_eyre::Result<()> {
//!     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000").await?;
//!
//!     let _ = wx.wait().await;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Example 2
//!
//! Below is an example of future-chaining for more advanced uses.
//!
//! ##### Barrier:
//!
//! ```no_run
//! use tsyncp::barrier;
//! use serde::{Serialize, Deserialize};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> color_eyre::Result<()> {
//!     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
//!         .limit(5)                       // Limit total number of allowed connections to 5.
//!         .set_tcp_nodelay(true)          // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)        // Set tcp option `reuseaddr` to `true`.
//!         .accept()
//!         .to_limit()                     // Accept connections up to the limit before returning.
//!         .await?;
//!
//!     tokio::time::sleep(Duration::from_millis(5000)).await;
//!
//!     bx.release().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ##### Waiter:
//!
//! ```no_run
//! use tsyncp::barrier;
//! use serde::{Serialize, Deserialize};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> color_eyre::Result<()> {
//!     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:8000")
//!         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
//!         .set_tcp_nodelay(true)                      // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)                    // Set tcp option `reuseaddr` to `true`.
//!         .await?;
//!
//!     let _ = wx.wait().await;
//!
//!     Ok(())
//! }
//! ```
//!
//! All configurable chain futures for `barrier_on` and `waiter_to` are in [builder] module.

use crate::{
    channel,
    multi_channel::{self, accept::AcceptFuture, send::SendFuture},
    util::{codec::EmptyCodec, listener::WriteListener, tcp, Accept},
};
use futures::Future;
use snafu::Backtrace;
use std::net::{SocketAddr, ToSocketAddrs};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
};

pub mod builder;

/// Method to initialize [Barrier]. Returns a [BarrierBuilderFuture](builder::BarrierBuilderFuture)
/// which awaiting it builds the barrier.
///
/// This future can also be chained to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [BarrierBuilderFuture](builder::BarrierBuilderFuture).
///
/// ```no_run
/// use tsyncp::barrier;
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000").await?;
///
///     // Accept 5 connections.
///     bx.accept().num(5).await?;
///
///     bx.release().await?;
///
///     Ok(())
/// }
/// ```
pub fn barrier_on<A: 'static + Clone + Send + ToSocketAddrs>(
    local_addr: A,
) -> builder::BarrierBuilderFuture<
    A,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<(), EmptyCodec>>>,
> {
    builder::new_barrier(local_addr)
}

/// Method to initialize [Waiter]. Returns a [WaiterBuilderFuture](builder::WaiterBuilderFuture)
/// which can be chained with other futures to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [WaiterBuilderFuture](builder::WaiterBuilderFuture).
///
/// ### Example:
///
/// ```no_run
/// use tsyncp::barrier;
/// use serde::{Serialize, Deserialize};
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut bx: barrier::Waiter = barrier::waiter_to("localhost:8000").await?;
///
///     let _ = bx.wait().await;
///
///     Ok(())
/// }
/// ```
pub fn waiter_to<A: 'static + Clone + Send + ToSocketAddrs>(
    dest: A,
) -> builder::WaiterBuilderFuture<
    A,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<(), EmptyCodec>>>,
> {
    builder::new_waiter(dest)
}

/// Single-connection waiter that waits for the [Barrier] to release.
///
/// [Skip to APIs](#implementations)
///
/// ### Example:
///
/// ```no_run
/// use color_eyre::Result;
/// use std::time::Duration;
/// use tsyncp::barrier;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:11114")
///         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
///         .await?;
///
///     let _ = wx.wait().await;
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Waiter<S = tcp::OwnedReadHalf>(#[pin] pub(crate) channel::Channel<(), EmptyCodec, S>);

impl<S> Waiter<S> {
    /// Returns local address
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    /// Returns peer address
    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<S> Waiter<S>
where
    S: AsyncRead + Unpin,
{
    /// Wait for [Barrier] to release.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use std::time::Duration;
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let mut wx: barrier::Waiter = barrier::waiter_to("localhost:11114")
    ///         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
    ///         .await?;
    ///
    ///     let _ = wx.wait().await;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn wait(&mut self) -> channel::recv::RecvFuture<'_, (), EmptyCodec, S> {
        self.0.recv()
    }
}

/// Multi-connection Barrier that can send data to all connected [Waiter]s.
///
/// [Skip to APIs](#implementations)
///
/// ## Contents
/// * [Example 1](#example-1-releasing-to-specific-addresses): Releasing to specific addresses.
/// * [Example 2](#example-2-releasing-by-filtering-addresses): Releasing by filtering addresses.
/// * [Example 3](#example-3-accepting-connections-while-releasing): Accepting connections
/// while releasing.
///
/// ### Example 1: Releasing to Specific Addresses
///
/// You can actually choose which addresses you release.
///
/// ```no_run
/// use color_eyre::Result;
/// use tsyncp::barrier;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     // Get all the addresses where the ports are even.
///     let addrs = bx
///         .peer_addrs()
///         .into_iter()
///         .filter(|a| a.port() % 2 == 0)
///         .collect::<Vec<_>>();
///
///     bx.release().to(&addrs).await?;
///
///     Ok(())
/// }
/// ```
///
/// ### Example 2: Releasing by Filtering Addresses
///
/// You can also release addresses that match certain criteria.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use tsyncp::barrier;
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     bx.release().filter(|a| a.port() % 2 == 0).await?;
/// #     Ok(())
/// # }
/// ```
///
/// ### Example 3: Accepting Connections While Releasing.
///
/// You can concurrently accept connections while releasing. (although why would you?)
///
/// When all release messages are flushed, the future will return the result along with the accepted addrs
/// in a tuple.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use tsyncp::barrier;
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:11114")
///         .accept()
///         .num(5)
///         .await?;
///
///     // accepting while waiting to release.
///     let (res, accept_res) = bx
///         .release()
///         .accepting()
///         .handle(|a| println!("{a} connected!"))
///         .await;
///
///     if let Ok(num) = accept_res {
///         println!("{num} connections accepted!");
///     }
///
///     // see if releasing was successful.
///     res?;
/// #     Ok(())
/// # }
/// ```
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Barrier<const N: usize = 0, L: Accept = WriteListener<TcpListener>>(
    #[pin] multi_channel::Channel<(), EmptyCodec, N, L>,
);

impl<const N: usize, L> AsMut<multi_channel::Channel<(), EmptyCodec, N, L>> for Barrier<N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut multi_channel::Channel<(), EmptyCodec, N, L> {
        &mut self.0
    }
}

impl<const N: usize, L> Barrier<N, L>
where
    L: Accept,
{
    /// Returns the currently connected number of waiters.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(bx.len(), 5);
    ///     println!("{} connections accepted!", bx.len());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns the total allowed number of waiters.
    ///
    /// Trying to accept connections when full will result in an error.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(bx.len(), 10);
    ///     println!("{} connections accepted!", bx.len());
    ///
    ///     assert_eq!(bx.limit(), Some(10));
    ///     println!("channel limit = {}", bx.limit().unwrap());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn limit(&self) -> Option<usize> {
        self.0.limit()
    }

    /// Returns whether the connection limit is reached.
    ///
    /// If there is no limit, it will always return `false`.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(bx.len(), 5);
    ///     println!("{} connections accepted!", bx.len());
    ///
    ///     assert!(!bx.is_full());
    ///
    ///     bx.accept().to_limit().await?;
    ///
    ///     assert_eq!(bx.limit(), Some(10));
    ///     assert_eq!(bx.len(), 10);
    ///     println!("channel limit = {}", bx.limit().unwrap());
    ///
    ///     assert!(bx.is_full());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn is_full(&self) -> bool {
        self.0.is_full()
    }

    /// Returns the local address this barrier is bound to.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000").await?;
    ///
    ///     println!("local_addr is {}", bx.local_addr());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn local_addr(&self) -> &SocketAddr {
        self.0.local_addr()
    }

    /// Returns vec of all connected remote addresses.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(bx.peer_addrs().len(), 10);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.0.peer_addrs()
    }
}

impl<const N: usize, L> Barrier<N, L>
where
    L: Accept,
{
    /// Returns an AcceptFuture which accepts a connection; this future can be chained to accept
    /// multiple connections, filter connections and handle accepted connections.
    ///
    /// See [AcceptFuture](crate::multi_channel::accept::AcceptFuture) for details.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:8000").await?;
    ///
    ///     // Accept 5 connections.
    ///     let num = bx
    ///     .accept()
    ///     .num(5)
    ///     .filter(|a| a.port() % 2 == 0)
    ///     .handle(|a| println!("{a} connected!"))
    ///     .await?;
    ///
    ///     println!("{num} connections accepted!");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn accept(
        &mut self,
    ) -> AcceptFuture<
        '_,
        (),
        EmptyCodec,
        N,
        L,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    > {
        self.0.accept()
    }
}

impl<const N: usize, L: Accept> Barrier<N, L>
where
    L::Output: AsyncWrite + Unpin,
{
    /// Returns [SendFuture](crate::multi_channel::send::SendFuture) that sends an empty tuple and flushes an item to all waiters.
    ///
    /// The future can be chained to release only specific addresses, filter addresses, and
    /// accept connections concurrently while waiting to send and flush items.
    ///
    /// To see the chain methods, see [SendFuture](crate::multi_channel::send::SendFuture).
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use tsyncp::barrier;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let mut bx: barrier::Barrier = barrier::barrier_on("localhost:11114")
    ///         .accept()
    ///         .num(10)
    ///         .await?;
    ///
    ///     bx.release().await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn release(&mut self) -> SendFuture<'_, (), EmptyCodec, N, L> {
        self.0.send(())
    }
}

#[allow(missing_docs)]
pub mod errors {
    use super::*;
    use snafu::Snafu;
    use std::io;

    #[derive(Debug, Snafu)]
    #[snafu(display("[BarrierError] Failed to release"))]
    #[snafu(visibility(pub(super)))]
    pub struct BarrierError {
        source: multi_channel::errors::SinkError<io::Error>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[WaiterError] Failed to wait"))]
    #[snafu(visibility(pub(super)))]
    pub struct WaiterError {
        source: channel::errors::StreamError<io::Error>,
        backtrace: Backtrace,
    }
}
