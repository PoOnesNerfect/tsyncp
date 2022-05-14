//! Single-producer/Multi-consumer API implementation.
//!
//! [Sender] is initialized by listening on a local address with [sender_on].
//!
//! [Receiver] is initialized by connecting to a remote address of the single producer with [receiver_to].
//!
//! [sender_on] and [receiver_to] can take any type of parameter that implements [ToSocketAddrs](std::net::ToSocketAddrs).
//!
//! This module contains few type alias for [Sender] and [Receiver]:
//! - [JsonSender] and [JsonReceiver]
//! - [ProstSender] and [ProstReceiver]
//! - [BincodeSender] and [BincodeReceiver]
//!
//! [Skip to APIs](#modules)
//!
//! ## Example Contents
//! These examples will demonstrate how to initialize a Sender and Receiver and how to chain configurations when initializing.
//! * [Example 1](#example-1): Basic initialization.
//! * [Example 2](#example-2): Chaining futures to initialization.
//!
//! To see how you can use a Sender once initialized, see [Sender's documentation](Sender).
//! To see how you can use a Receiver once initialized, see [Receiver's documentation](Receiver).
//!
//! ### Example 1
//!
//! Below is the basic example of initializing, accepting, and receiving data with
//! [JsonSender]. If you want to use a receiver with a different codec, just replace `JsonSender<Dummy>`
//! with `ProstSender<Dummy>`, for example, or `Sender<Dummy, CustomCodec>`.
//!
//! ##### Sender:
//!
//! ```no_run
//! use tsyncp::broadcast;
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
//!     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000").await?;
//!
//!     let dummy = Dummy {
//!         field1: String::from("hello world"),
//!         field2: 123123,
//!         field3: vec![1, 2, 3, 4]
//!     };
//!
//!     tx.send(dummy).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ##### Receiver:
//!
//! ```no_run
//! use tsyncp::broadcast;
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
//!     let mut rx: broadcast::JsonReceiver<Dummy> = broadcast::receiver_to("localhost:8000").await?;
//!
//!     // Receive some item on the receiver.
//!     if let Some(Ok(item)) = rx.recv().await {
//!         println!("received {item:?}");
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
//! ##### Sender:
//!
//! ```no_run
//! use tsyncp::broadcast;
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
//!     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000")
//!         .limit(5)                       // Limit total number of allowed connections to 5.
//!         .set_tcp_nodelay(true)          // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)        // Set tcp option `reuseaddr` to `true`.
//!         .accept()
//!         .to_limit()                     // Accept connections up to the limit before returning.
//!         .await?;
//!
//!     let dummy = Dummy {
//!         field1: String::from("hello world"),
//!         field2: 123123,
//!         field3: vec![1, 2, 3, 4]
//!     };
//!
//!     tx.send(dummy).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ##### Sender:
//!
//! ```no_run
//! use tsyncp::broadcast;
//! use serde::{Serialize, Deserialize};
//! use std::time::Duration;
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
//!     let mut rx: broadcast::JsonReceiver<Dummy> = broadcast::receiver_to("localhost:8000")
//!         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
//!         .set_tcp_nodelay(true)                      // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)                    // Set tcp option `reuseaddr` to `true`.
//!         .await?;
//!
//!     // Receive some item on the receiver.
//!     if let Some(Ok(item)) = rx.recv().await {
//!         println!("received {item:?}");
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! All configurable chain futures for `sender_on` and `receiver_to` are in [builder] module.

use crate::{
    channel,
    multi_channel::{self, accept::AcceptFuture, send::SendFuture},
    util::{
        codec::{DecodeMethod, EncodeMethod},
        listener::WriteListener,
        tcp, Accept,
    },
};
use futures::Future;
use std::net::{SocketAddr, ToSocketAddrs};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
};

pub mod builder;

/// Type alias for `Receiver<T, tsyncp::util::codec::JsonCodec>`.
#[cfg(feature = "json")]
pub type JsonReceiver<T> = Receiver<T, crate::util::codec::JsonCodec>;

/// Type alias for `Sender<T, tsyncp::util::codec::JsonCodec>`.
#[cfg(feature = "json")]
pub type JsonSender<T, const N: usize = 0> = Sender<T, crate::util::codec::JsonCodec, N>;

/// Type alias for `Receiver<T, tsyncp::util::codec::ProstCodec>`.
#[cfg(feature = "prost")]
pub type ProstReceiver<T> = Receiver<T, crate::util::codec::ProstCodec>;

/// Type alias for `Sender<T, tsyncp::util::codec::ProstCodec>`.
#[cfg(feature = "prost")]
pub type ProstSender<T, const N: usize = 0> = Sender<T, crate::util::codec::ProstCodec, N>;

/// Type alias for `Receiver<T, tsyncp::util::codec::BincodeCodec>`.
#[cfg(feature = "bincode")]
pub type BincodeReceiver<T> = Receiver<T, crate::util::codec::BincodeCodec>;

/// Type alias for `Sender<T, tsyncp::util::codec::BincodeCodec>`.
#[cfg(feature = "bincode")]
pub type BincodeSender<T, const N: usize = 0> = Sender<T, crate::util::codec::BincodeCodec, N>;

// #[cfg(feature = "rkyv")]
// pub type RkyvReceiver<T> = Receiver<T, crate::util::codec::RkyvCodec>;

// #[cfg(feature = "rkyv")]
// pub type RkyvSender<T, const N: usize = 0> = Sender<T, crate::util::codec::RkyvCodec, N>;

/// Method to initialize [Sender]. Returns a [SenderBuilderFuture](builder::SenderBuilderFuture)
/// which awaiting it builds the sender.
///
/// This future can also be chained to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [SenderBuilderFuture](builder::SenderBuilderFuture).
///
/// ```no_run
/// use tsyncp::broadcast;
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
///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000").await?;
///
///     // Accept 5 connections before receiving `Dummy` items.
///     tx.accept().num(5).await?;
///
///     let dummy = Dummy {
///         field1: String::from("hello world"),
///         field2: 1234567,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     tx.send(dummy).await?;
///
///     Ok(())
/// }
/// ```
pub fn sender_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::SenderBuilderFuture<
    A,
    T,
    E,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E>>>,
> {
    builder::new_sender(local_addr)
}

/// Method to initialize [Receiver]. Returns a [ReceiverBuilderFuture](builder::ReceiverBuilderFuture)
/// which can be chained with other futures to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [ReceiverBuilderFuture](builder::ReceiverBuilderFuture).
///
/// ### Example:
///
/// ```no_run
/// use tsyncp::broadcast;
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
///     let mut rx: broadcast::JsonReceiver<Dummy> = broadcast::receiver_to("localhost:8000").await?;
///
///     // Receive some item on the channel.
///     if let Some(Ok(item)) = rx.recv().await {
///         println!("received {item:?}");
///     }
///
///     Ok(())
/// }
/// ```
pub fn receiver_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::ReceiverBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::new_receiver(dest)
}

/// Single-connection message receiver that can receiver data from a sender connection.
///
/// [Skip to APIs](#implementations)
///
/// ### Example:
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::broadcast;
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
///     let mut rx: broadcast::JsonReceiver<Dummy> = broadcast::receiver_to("localhost:11114")
///         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
///         .await?;
///
///     // Receive some item on the channel.
///     if let Some(Ok(item)) = rx.recv().await {
///         println!("received {item:?}");
///     }
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Receiver<T, E, S = tcp::OwnedReadHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Receiver<T, E, S> {
    fn from(c: channel::Channel<T, E, S>) -> Self {
        Self(c)
    }
}

impl<T, E, S> From<Receiver<T, E, S>> for channel::Channel<T, E, S> {
    fn from(c: Receiver<T, E, S>) -> Self {
        c.0
    }
}

impl<T, E, S> Receiver<T, E, S> {
    /// Returns local address
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    /// Returns peer address
    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E: DecodeMethod<T>, S: AsyncRead + Unpin> Receiver<T, E, S> {
    /// Returns [RecvFuture](crate::channel::recv::RecvFuture) which asynchronously receives an item.
    ///
    /// To extend this method by chaining futures, see [RecvFuture](crate::channel::recv::RecvFuture).
    pub fn recv(&mut self) -> crate::channel::recv::RecvFuture<'_, T, E, S> {
        self.0.recv()
    }
}

/// Multi-connection message sender that can send data to all connected receivers.
///
/// [Skip to APIs](#implementations)
///
/// ## Generic Parameters
///
/// Currently, `Sender` has 4 generic paramters, which 2 of them are defaulted. I will go over them real quick.
///
/// First paramter, `T`, as you could guess, is the data type this sender is sending.
/// Technically, we could make `Sender` send any item without having to specify its type; however,
/// specifying the type as part of the sender is better because it clears up what the sender
/// expects to send. If the sender could send any item type, what type should the receiver receive?
///
/// Second parameter, `E`, is the codec type; here, you can specify anything that implements [EncodeMethod](crate::util::codec::EncodeMethod)
/// and [DecodeMethod](crate::util::codec::DecodeMethod). For Json, Protobuf, and Bincode, there
/// are already type aliases called `JsonChannel<T>`, `ProstChannel<T>`, and `BincodeChannel<T>`
/// which is a little easier than typing `sender<T, tsyncp::util::codec::JsonCodec>`, and so on.
///
/// Third paramter, `const N: usize = 0`, is defaulted to 0, so the user does not have to specify
/// it unless necessary. This paramter represents the length of the array used for the [StreamPool](crate::util::stream_pool::StreamPool)
/// if the user chooses to use an array instead of a vec. If `N` is `0` as defaulted, the stream
/// pool initializes with a vec; however, if it is specified to a value greater than 0, through
/// [SenderBuilderFuture::limit_const()](builder::SenderBuilderFuture::limit_const), then the stream pool
/// will be initialized as an array. For more in detail, see [StreamPool](crate::util::stream_pool::StreamPool)
/// and [SenderBuilderFuture::limit_const()](builder::SenderBuilderFuture::limit_const).
///
/// Fourth parameter, `L`, is a listener type that implements [Accept](crate::util::Accept)
/// trait. The user will not have to interact with parameter ever.
///
/// ## Gettings Started
///
/// Since initializing the sender is demonstrated in [broadcast](crate::broadcast) module,
/// we will go over some methods you can use with Sender.
///
/// ## Contents
/// * [Example 1](#example-1-sending-items-to-specific-addresses): Sending items to specific addresses.
/// * [Example 2](#example-2-sending-items-by-filtering-addresses): Sending items by filtering addresses.
/// * [Example 3](#example-3-accepting-connections-while-sending-items): Accepting connections
/// while sending items.
/// * [Example 4](#example-4-chaining-all-futures-to-send): Chaining all futures to `send`.
///
/// ### Example 1: Sending Items to Specific Addresses
///
/// You can actually choose which addresses you can send the items to.
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::broadcast;
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
///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     let dummy = Dummy {
///         field1: String::from("hello world"),
///         field2: 1234567,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     // Get all the addresses where the ports are even.
///     let addrs = tx
///         .peer_addrs()
///         .into_iter()
///         .filter(|a| a.port() % 2 == 0)
///         .collect::<Vec<_>>();
///
///     // Send to only the addresses in `addrs` vec.
///     tx.send(dummy).to(&addrs).await?;
///
///     Ok(())
/// }
/// ```
///
/// ### Example 2: Sending Items by Filtering Addresses
///
/// You can also send to addresses that match certain criteria.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::broadcast;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     let dummy = Dummy {
///         field1: String::from("hello world"),
///         field2: 1234567,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     // Send to only the addresses by filtering the address ports to even values.
///     tx.send(dummy).filter(|a| a.port() % 2 == 0).await?;
/// #     Ok(())
/// # }
/// ```
///
/// ### Example 3: Accepting Connections While Sending Items.
///
/// You can concurrently accept connections while sending items.
///
/// When item is sent and flushed, the future will return the result along with the accepted addrs
/// in a tuple.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::broadcast;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:11114")
///         .accept()
///         .num(5)
///         .await?;
///
///     let dummy = Dummy {
///         field1: String::from("hello world"),
///         field2: 1234567,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     // accepting while waiting to send.
///     let (res, accept_res) = tx
///         .send(dummy)
///         .accepting()
///         .handle(|a| println!("{a} connected!"))
///         .await;
///
///     if let Ok(num) = accept_res {
///         println!("{num} connections accepted!");
///     }
///
///     // see if sending was successful.
///     res?;
/// #     Ok(())
/// # }
/// ```
///
/// ### Example 4: Chaining All Futures to Send
///
/// Now, let's try chaining all the futures we learned so far.
///
/// We can either only use `to` or `filter`, so we'll just pick `filter`.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::broadcast;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     let dummy = Dummy {
///         field1: String::from("hello world"),
///         field2: 1234567,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     let (res, accept_res) = tx.send(dummy)
///         .filter(|a| a.port() % 2 == 0)      // Only send to addresses with even ports.
///         .accepting()                        // Accept while waiting to send item.
///         .num(5)                             // Set accepting number of connections to 5.
///         .filter(|a| a.port() % 2 == 1)      // Accept only odd port addresses.
///         .handle(|a| println!("{a} connected!"))
///         .await;
///
///     if let Ok(num) = accept_res {
///         println!("{num} connections accepted!");
///     }
///
///     // see if sending was successful.
///     res?;
/// #     Ok(())
/// # }
/// ```
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Sender<T, E, const N: usize = 0, L: Accept = WriteListener<TcpListener>>(
    #[pin] multi_channel::Channel<T, E, N, L>,
);

impl<T, E, const N: usize, L> AsMut<multi_channel::Channel<T, E, N, L>> for Sender<T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut multi_channel::Channel<T, E, N, L> {
        &mut self.0
    }
}

impl<T, E, const N: usize, L> From<multi_channel::Channel<T, E, N, L>> for Sender<T, E, N, L>
where
    L: Accept,
{
    fn from(c: multi_channel::Channel<T, E, N, L>) -> Self {
        Self(c)
    }
}

impl<T, E, const N: usize, L> From<Sender<T, E, N, L>> for multi_channel::Channel<T, E, N, L>
where
    L: Accept,
{
    fn from(c: Sender<T, E, N, L>) -> Self {
        c.0
    }
}

impl<T, E, const N: usize, L> Sender<T, E, N, L>
where
    L: Accept,
{
    /// Returns the currently connected number of receivers.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::broadcast;
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
    ///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000")
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(tx.len(), 5);
    ///     println!("{} connections accepted!", tx.len());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns the total allowed number of receivers.
    ///
    /// Trying to accept connections when full will result in an error.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::broadcast;
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
    ///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(tx.len(), 10);
    ///     println!("{} connections accepted!", tx.len());
    ///
    ///     assert_eq!(tx.limit(), Some(10));
    ///     println!("channel limit = {}", tx.limit().unwrap());
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
    /// use tsyncp::broadcast;
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
    ///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(tx.len(), 5);
    ///     println!("{} connections accepted!", tx.len());
    ///
    ///     assert!(!tx.is_full());
    ///
    ///     tx.accept().to_limit().await?;
    ///
    ///     assert_eq!(tx.limit(), Some(10));
    ///     assert_eq!(tx.len(), 10);
    ///     println!("channel limit = {}", tx.limit().unwrap());
    ///
    ///     assert!(tx.is_full());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn is_full(&self) -> bool {
        self.0.is_full()
    }

    /// Returns the local address this sender is bound to.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::broadcast;
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
    ///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000").await?;
    ///
    ///     println!("local_addr is {}", tx.local_addr());
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
    /// use tsyncp::broadcast;
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
    ///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(tx.peer_addrs().len(), 10);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.0.peer_addrs()
    }
}

impl<T, E, const N: usize, L> Sender<T, E, N, L>
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
    /// use tsyncp::broadcast;
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
    ///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:8000").await?;
    ///
    ///     // Accept 5 connections.
    ///     let num = tx
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
    ) -> AcceptFuture<'_, T, E, N, L, impl FnMut(SocketAddr), impl FnMut(SocketAddr) -> bool> {
        self.0.accept()
    }
}

impl<T, E, const N: usize, L> Sender<T, E, N, L>
where
    E: EncodeMethod<T>,
    L: Accept,
    L::Output: AsyncWrite + Unpin,
{
    /// Returns [SendFuture](crate::multi_channel::send::SendFuture) that sends and flushes an item to all receivers.
    ///
    /// The future can be chained to send item to only specific addresses, filter addresses, and
    /// accept connections concurrently while waiting to send and flush items.
    ///
    /// To see the chain methods, see [SendFuture](crate::multi_channel::send::SendFuture).
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::broadcast;
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
    ///     let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:11114")
    ///         .accept()
    ///         .num(10)
    ///         .await?;
    ///
    ///     let dummy = Dummy {
    ///         field1: String::from("hello world"),
    ///         field2: 1234567,
    ///         field3: vec![1, 2, 3, 4]
    ///     };
    ///
    ///     tx.send(dummy).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn send(&mut self, item: T) -> SendFuture<'_, T, E, N, L> {
        self.0.send(item)
    }

    /// Renamed method alias for [send](Sender::send).
    pub fn broadcast(&mut self, item: T) -> SendFuture<'_, T, E, N, L> {
        self.send(item)
    }
}
