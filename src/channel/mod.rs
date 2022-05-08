//! Generic single-connection channel for sending and receiving items.
//!
//! [Channel] is initialized by connecting to a remote address, or listening to a local_address
//! until a single connection is made.
//! [Channel] can be initialized with [channel_to] or [channel_on].
//!
//! [channel_to] can take any type of parameter that implements [ToSocketAddrs](std::net::ToSocketAddrs).
//!
//! This module also contains the following modules:
//! - [builder] module: Contains [BuilderFuture](builder::BuilderFuture) used to chaining and building
//! a Channel.
//! - [recv] module: Contains [RecvFuture](recv::RecvFuture), used to receive items,
//!
//! [Skip to APIs](#modules)
//!
//! ## Example Contents
//! These examples will demonstrate how to initialize a Channel and how to chain configurations when initializing.
//! * [Example 1](#example-1): Basic initialization.
//! * [Example 2](#example-2): Chaining futures to initialization.
//!
//! To see how you can use the Channel once initialized, see [Channel's documentation](Channel).
//!
//! ### Example 1
//!
//! Below is the basic example of initializing, accepting, receiving and sending data with
//! [JsonChannel]. If you want to use a channel with a different codec, just replace `JsonChannel<Dummy>`
//! with `ProstChannel<Dummy>`, for example, or `Channel<Dummy, CustomCodec>`.
//!
//! ```no_run
//! use tsyncp::channel;
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct Dummy {
//!     field1: String,
//!     field2: u64,
//!     field3: Vec<u8>,
//! }
//!
//! #[tokio::main]
//! async fn main() -> color_eyre::Result<()> {
//!     let mut ch: channel::JsonChannel<Dummy> = channel::channel_to("localhost:8000").await?;
//!
//!     // Receive some item on the channel.
//!     if let Some(Ok(item)) = ch.recv().await {
//!         println!("received {item:?}");
//!
//!         // Broadcast received item to all connections.
//!         ch.send(item).await?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Example 2
//!
//! Below is an example of future-chaining for more advanced uses.
//!
//! ```no_run
//! use tsyncp::channel;
//! use serde::{Serialize, Deserialize};
//! use std::time::Duration;
//!
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> color_eyre::Result<()> {
//!     let mut ch: channel::JsonChannel<Dummy> = channel::channel_on("127.0.0.1:8000")
//!         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
//!         .set_tcp_nodelay(true)                      // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)                    // Set tcp option `reuseaddr` to `true`.
//!         .await?;
//!
//!     // Receive an item.
//!     if let Some(Ok(item)) = ch.recv().await {
//!         println!("received {item:?}");
//!
//!         // send back received item.
//!         ch.send(item).await?;
//!     }
//!
//! #     Ok(())
//! # }
//! ```
//!
//! All configurable chain futures for `channel_to` are in [builder] module.

use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::{Framed, Split};
use crate::{broadcast, mpsc};
use errors::*;
use futures::{ready, Future, Sink, SinkExt};
use snafu::{ensure, Backtrace, ResultExt};
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

pub mod builder;
pub mod recv;

/// Type alias for `Channel<T, tsyncp::util::codec::JsonCodec>`.
#[cfg(feature = "json")]
pub type JsonChannel<T> = Channel<T, crate::util::codec::JsonCodec>;

/// Type alias for `Channel<T, tsyncp::util::codec::ProstCodec>`.
#[cfg(feature = "protobuf")]
pub type ProstChannel<T> = Channel<T, crate::util::codec::ProstCodec>;

/// Type alias for `Channel<T, tsyncp::util::codec::BincodeCodec>`.
#[cfg(feature = "bincode")]
pub type BincodeChannel<T> = Channel<T, crate::util::codec::BincodeCodec>;

// #[cfg(feature = "rkyv")]
// pub type RkyvChannel<T> = Channel<T, crate::util::codec::RkyvCodec>;

/// Method to initialize [Channel]. Returns a [BuilderFuture](builder::BuilderFuture)
/// which can be chained with other futures to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [BuilderFuture](builder::BuilderFuture).
///
/// ### Example:
///
/// ```no_run
/// use tsyncp::channel;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct Dummy {
///     field1: String,
///     field2: u64,
///     field3: Vec<u8>,
/// }
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut ch: channel::JsonChannel<Dummy> = channel::channel_to("localhost:8000").await?;
///
///     // Receive some item on the channel.
///     if let Some(Ok(item)) = ch.recv().await {
///         println!("received {item:?}");
///
///         // Broadcast received item to all connections.
///         ch.send(item).await?;
///     }
///
///     Ok(())
/// }
/// ```
pub fn channel_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::BuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = builder::Result<Channel<T, E>>>,
> {
    builder::new(dest, false)
}

/// Method to initialize [Channel]. Returns a [BuilderFuture](builder::BuilderFuture)
/// which can be chained with other futures to configure the initialization.
///
/// This method binds and listens to the given address, and returns Channel after accepting a
/// single connection.
///
/// To see how you can chain more configurations to this method, see [BuilderFuture](builder::BuilderFuture).
///
/// ### Example:
///
/// ```no_run
/// use tsyncp::channel;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct Dummy {
///     field1: String,
///     field2: u64,
///     field3: Vec<u8>,
/// }
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut ch: channel::JsonChannel<Dummy> = channel::channel_on("localhost:8000").await?;
///
///     // Receive some item on the channel.
///     if let Some(Ok(item)) = ch.recv().await {
///         println!("received {item:?}");
///
///         // Broadcast received item to all connections.
///         ch.send(item).await?;
///     }
///
///     Ok(())
/// }
/// ```
pub fn channel_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::BuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = builder::Result<Channel<T, E>>>,
> {
    builder::new(local_addr, true)
}

/// Single-connection channel that can send/receive data from the connection.
///
/// Since initializing the channel is demonstrated in [channel](crate::channel) module,
/// we will go over some methods you can use with Channel.
///
/// Also, you can [split](Channel::split) the channel into [broadcast::Receiver](crate::broadcast::Receiver)
/// and [mpsc::Sender](crate::broadcast::Sender) pair to send and receive data concurrently.
/// See example for this in [Example 3](#example-3-concurrently-send-and-recv-data).
///
/// [Skip to APIs](#implementations)
///
/// ## Example Contents
/// * [Example 1](#example-1-sending-items): Sending items.
/// * [Example 2](#example-2-receiving-items-as-bytes): Receiving items as bytes.
/// * [Example 3](#example-3-concurrently-send-and-recv-data): Concurrently send and recv data.
///
/// ### Example 1: Sending Items
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::channel;
/// use std::time::Duration;
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct Dummy {
///     field1: String,
///     field2: u64,
///     field3: Vec<u8>,
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let mut ch: channel::JsonChannel<Dummy> = channel::channel_to("localhost:11114")
///         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
///         .await?;
///
///     let dummy = Dummy {
///         field1: String::from("hello world"),
///         field2: 1234567,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     ch.send(dummy).await?;
///
///     Ok(())
/// }
/// ```
///
/// ### Example 2: Receiving Items as Bytes
///
/// You can receive data as raw bytes before it gets encoded. (if you wanted raw json bytes, for
/// example)
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::channel;
/// # use std::time::Duration;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: channel::JsonChannel<Dummy> = channel::channel_to("localhost:11114")
///         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
///         .await?;
///
///     /// Receive item and return before decoding it as `Dummy` struct.
///     if let Some(Ok(bytes)) = ch.recv().as_bytes().await {
///         /// Print raw json bytes as json object.
///         println!("json object: {}", std::str::from_utf8(&bytes).unwrap());
///     }
/// #     Ok(())
/// # }
/// ```
///
/// ### Example 3: Concurrently Send and Recv Data.
///
/// You can split the channel to send and receive items at the same time.
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::{channel, mpsc, broadcast};
/// use std::time::Duration;
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct Dummy {
///     field1: String,
///     field2: u64,
///     field3: Vec<u8>,
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let ch: channel::JsonChannel<Dummy> = channel::channel_to("localhost:11114")
///         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
///         .await?;
///
///     let (rx, tx) = ch.split();
///
///     // Below two lines are unnecessary, they are just to show the types `split` returns.
///     let mut rx: broadcast::JsonReceiver<Dummy> = rx;
///     let mut tx: mpsc::JsonSender<Dummy> = tx;
///
///     tokio::spawn(async move {
///         while let Some(Ok(item)) = rx.recv().await {
///             println!("received {item:?}");
///         }
///     });
///
///     for i in 0..1_000_000 {
///         let dummy = Dummy {
///             field1: String::from("hello world"),
///             field2: i,
///             field3: vec![1, 2, 3, 4]
///         };
///
///         tx.send(dummy).await?;
///     }
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Channel<T, E, S = TcpStream> {
    #[pin]
    framed: Framed<S>,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    _phantom: PhantomData<(T, E)>,
}

impl<T, E, S> Channel<T, E, S> {
    /// Returns local addr
    pub fn local_addr(&self) -> &SocketAddr {
        &self.local_addr
    }

    /// Returns peer addr
    pub fn peer_addr(&self) -> &SocketAddr {
        &self.peer_addr
    }
}

impl<T, E, S> Channel<T, E, S>
where
    S: Split,
{
    /// Splits the Channel into a pair of [broadcast::Receiver](crate::broadcast::Receiver)
    /// and [mpsc::Sender](crate::mpsc::Sender).
    pub fn split(
        self,
    ) -> (
        broadcast::Receiver<T, E, S::Left>,
        mpsc::Sender<T, E, S::Right>,
    ) {
        Split::split(self)
    }
}

impl<T, E, S> Split for Channel<T, E, S>
where
    S: Split,
{
    type Left = broadcast::Receiver<T, E, S::Left>;
    type Right = mpsc::Sender<T, E, S::Right>;
    type Error = UnsplitError<<S as Split>::Error>;

    fn split(self) -> (Self::Left, Self::Right) {
        let Channel {
            framed,
            local_addr,
            peer_addr,
            ..
        } = self;

        let (r, w) = framed.split();

        let r = Channel {
            framed: r,
            local_addr,
            peer_addr,
            _phantom: PhantomData,
        };

        let w = Channel {
            framed: w,
            local_addr,
            peer_addr,
            _phantom: PhantomData,
        };

        (r.into(), w.into())
    }

    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error> {
        let Channel {
            framed: l_framed,
            local_addr: l_local_addr,
            peer_addr: l_peer_addr,
            ..
        } = left.into();

        let Channel {
            framed: r_framed,
            local_addr: r_local_addr,
            peer_addr: r_peer_addr,
            ..
        } = right.into();

        ensure!(
            l_local_addr == r_local_addr,
            UnequalLocalAddrSnafu {
                l_local_addr,
                r_local_addr
            }
        );

        ensure!(
            l_peer_addr == r_peer_addr,
            UnequalPeerAddrSnafu {
                l_peer_addr,
                r_peer_addr
            }
        );

        let framed = <_>::unsplit(l_framed, r_framed).context(FramedUnsplitSnafu)?;

        Ok(Channel {
            framed,
            local_addr: l_local_addr,
            peer_addr: l_peer_addr,
            _phantom: PhantomData,
        })
    }
}

impl<T, E, S> Channel<T, E, S>
where
    E: EncodeMethod<T>,
    S: AsyncWrite + Unpin,
{
    /// Asynchronously send item to the peer addr
    pub async fn send(&mut self, item: T) -> Result<(), SinkError<E::Error>> {
        SinkExt::send(self, item).await
    }
}

impl<T, E: EncodeMethod<T>, S> Sink<T> for Channel<T, E, S>
where
    S: AsyncWrite + Unpin,
{
    type Error = SinkError<E::Error>;

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu {
            addr: self.local_addr,
            peer_addr: self.peer_addr,
        })?;

        self.framed
            .start_send_unpin(encoded)
            .context(StartSendSnafu {
                addr: self.local_addr,
                peer_addr: self.peer_addr,
            })?;

        Ok(())
    }

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.framed.poll_ready_unpin(cx)).context(PollReadySnafu {
            addr: self.local_addr,
            peer_addr: self.peer_addr,
        });

        Poll::Ready(res)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.framed.poll_flush_unpin(cx)).context(PollFlushSnafu {
            addr: self.local_addr,
            peer_addr: self.peer_addr,
        });

        Poll::Ready(res)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.framed.poll_close_unpin(cx)).context(PollCloseSnafu {
            addr: self.local_addr,
            peer_addr: self.peer_addr,
        });

        Poll::Ready(res)
    }
}

impl<T, E: DecodeMethod<T>, S> Channel<T, E, S>
where
    S: AsyncRead + Unpin,
{
    /// Returns [RecvFuture](recv::RecvFuture) which asynchronously receives an item.
    pub fn recv(&mut self) -> recv::RecvFuture<'_, T, E, S> {
        recv::RecvFuture::new(self)
    }
}

pub mod errors {
    use super::*;
    use crate::util::frame_codec::errors::CodecError;
    use snafu::Snafu;
    use std::io::ErrorKind;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SinkError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[SinkError] Failed to encode item on {addr} to {peer_addr}"))]
        ItemEncode {
            addr: SocketAddr,
            peer_addr: SocketAddr,
            source: E,
            backtrace: Backtrace,
        },
        #[snafu(display("[SinkError] Failed start_send on {addr} to {peer_addr}"))]
        StartSend {
            addr: SocketAddr,
            peer_addr: SocketAddr,
            source: CodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[SinkError] Failed poll_ready on {addr} to {peer_addr}"))]
        PollReady {
            addr: SocketAddr,
            peer_addr: SocketAddr,
            source: CodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[SinkError] Failed poll_flush on {addr} to {peer_addr}"))]
        PollFlush {
            addr: SocketAddr,
            peer_addr: SocketAddr,
            source: CodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[SinkError] Failed poll_close on {addr} to {peer_addr}"))]
        PollClose {
            addr: SocketAddr,
            peer_addr: SocketAddr,
            source: CodecError,
            backtrace: Backtrace,
        },
    }

    impl<E> SinkError<E>
    where
        E: 'static + snafu::Error,
    {
        pub fn local_addr(&self) -> &SocketAddr {
            match self {
                Self::StartSend { addr, .. } => addr,
                Self::PollReady { addr, .. } => addr,
                Self::PollFlush { addr, .. } => addr,
                Self::PollClose { addr, .. } => addr,
                Self::ItemEncode { addr, .. } => addr,
            }
        }

        pub fn peer_addr(&self) -> &SocketAddr {
            match self {
                Self::StartSend { peer_addr, .. } => peer_addr,
                Self::PollReady { peer_addr, .. } => peer_addr,
                Self::PollFlush { peer_addr, .. } => peer_addr,
                Self::PollClose { peer_addr, .. } => peer_addr,
                Self::ItemEncode { peer_addr, .. } => peer_addr,
            }
        }

        pub fn is_encode_error(&self) -> bool {
            matches!(self, Self::ItemEncode { .. })
        }

        pub fn is_sink_error(&self) -> bool {
            !self.is_encode_error()
        }

        pub fn as_io(&self) -> Option<&std::io::Error> {
            match self {
                Self::StartSend { source, .. } => source.as_io(),
                Self::PollReady { source, .. } => source.as_io(),
                Self::PollFlush { source, .. } => source.as_io(),
                Self::PollClose { source, .. } => source.as_io(),
                _ => None,
            }
        }

        /// Check if the error is a connection error.
        ///
        /// Returns `true` if the error either `reset`, `refused`, `aborted`, `not connected`, or
        /// `broken pipe`.
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn is_connection_error(&self) -> bool {
            self.is_connection_reset()
                || self.is_connection_refused()
                || self.is_connection_aborted()
                || self.is_not_connected()
                || self.is_broken_pipe()
        }

        pub fn is_connection_reset(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionReset)
                .unwrap_or_default()
        }

        pub fn is_connection_refused(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionRefused)
                .unwrap_or_default()
        }

        pub fn is_connection_aborted(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionAborted)
                .unwrap_or_default()
        }

        pub fn is_not_connected(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::NotConnected)
                .unwrap_or_default()
        }

        pub fn is_broken_pipe(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::BrokenPipe)
                .unwrap_or_default()
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum StreamError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[StreamError] Failed to decode frame of data on {addr} to {peer_addr}"))]
        ItemDecode {
            addr: SocketAddr,
            peer_addr: SocketAddr,
            source: E,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamError] Failed to call poll_next on {addr} to {peer_addr}"))]
        PollNext {
            addr: SocketAddr,
            peer_addr: SocketAddr,
            source: CodecError,
            backtrace: Backtrace,
        },
    }

    impl<E> StreamError<E>
    where
        E: 'static + snafu::Error,
    {
        pub fn local_addr(&self) -> &SocketAddr {
            match self {
                Self::PollNext { addr, .. } => addr,
                Self::ItemDecode { addr, .. } => addr,
            }
        }

        pub fn peer_addr(&self) -> &SocketAddr {
            match self {
                Self::ItemDecode { peer_addr, .. } => peer_addr,
                Self::PollNext { peer_addr, .. } => peer_addr,
            }
        }

        pub fn is_decode_error(&self) -> bool {
            matches!(self, Self::ItemDecode { .. })
        }

        pub fn is_recv_error(&self) -> bool {
            !self.is_decode_error()
        }

        pub fn as_io(&self) -> Option<&std::io::Error> {
            match self {
                Self::PollNext { source, .. } => source.as_io(),
                _ => None,
            }
        }

        /// Check if the error is a connection error.
        ///
        /// Returns `true` if the error either `reset`, `refused`, `aborted`, `not connected`, or
        /// `broken pipe`.
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn is_connection_error(&self) -> bool {
            self.is_connection_reset()
                || self.is_connection_refused()
                || self.is_connection_aborted()
                || self.is_not_connected()
                || self.is_broken_pipe()
        }

        pub fn is_connection_reset(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionReset)
                .unwrap_or_default()
        }

        pub fn is_connection_refused(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionRefused)
                .unwrap_or_default()
        }

        pub fn is_connection_aborted(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionAborted)
                .unwrap_or_default()
        }

        pub fn is_not_connected(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::NotConnected)
                .unwrap_or_default()
        }

        pub fn is_broken_pipe(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::BrokenPipe)
                .unwrap_or_default()
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum UnsplitError<E: 'static + snafu::Error> {
        #[snafu(display("[UnsplitError] Underlying channels' local addrs are different: {l_local_addr:?} != {r_local_addr:?}"))]
        UnequalLocalAddr {
            l_local_addr: SocketAddr,
            r_local_addr: SocketAddr,
        },
        #[snafu(display("[UnsplitError] Underlying channels' peer addrs are different: {l_peer_addr:?} != {r_peer_addr:?}"))]
        UnequalPeerAddr {
            l_peer_addr: SocketAddr,
            r_peer_addr: SocketAddr,
        },
        #[snafu(display("[UnsplitError] Failed to unsplit framed"))]
        FramedUnsplit { source: E, backtrace: Backtrace },
    }
}
