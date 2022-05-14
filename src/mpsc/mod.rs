//! Multi-producer/Single-consumer API implementation.
//!
//! [Sender] is initialized by connecting to a remote address of the single consumer with [sender_to].
//!
//! [Receiver] is initialized by listening on a local address with [receiver_on].
//!
//! [sender_to] and [receiver_on] can take any type of parameter that implements [ToSocketAddrs](std::net::ToSocketAddrs).
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
//! [JsonReceiver]. If you want to use a receiver with a different codec, just replace `JsonReceiver<Dummy>`
//! with `ProstReceiver<Dummy>`, for example, or `Receiver<Dummy, CustomCodec>`.
//!
//! ##### Receiver:
//!
//! ```no_run
//! use tsyncp::mpsc;
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
//!     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000").await?;
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
//! ##### Sender:
//!
//! ```no_run
//! use tsyncp::mpsc;
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
//!     let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:8000").await?;
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
//! ### Example 2
//!
//! Below is an example of future-chaining for more advanced uses.
//!
//! ##### Receiver:
//!
//! ```no_run
//! use tsyncp::mpsc;
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
//!     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
//!         .limit(5)                       // Limit total number of allowed connections to 5.
//!         .set_tcp_nodelay(true)          // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)        // Set tcp option `reuseaddr` to `true`.
//!         .accept()
//!         .to_limit()                     // Accept connections up to the limit before returning.
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
//! ##### Sender:
//!
//! ```no_run
//! use tsyncp::mpsc;
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
//!     let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:8000")
//!         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
//!         .set_tcp_nodelay(true)                      // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)                    // Set tcp option `reuseaddr` to `true`.
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
//! All configurable chain futures for `sender_to` and `receiver_on` are in [builder] module.

use crate::{
    channel,
    multi_channel::{self, accept::AcceptFuture, recv::RecvFuture},
    util::{
        codec::{DecodeMethod, EncodeMethod},
        listener::ReadListener,
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

/// Type alias for `Sender<T, tsyncp::util::codec::JsonCodec>`.
#[cfg(feature = "json")]
pub type JsonSender<T> = Sender<T, crate::util::codec::JsonCodec>;

/// Type alias for `Receiver<T, tsyncp::util::codec::JsonCodec>`.
#[cfg(feature = "json")]
pub type JsonReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::JsonCodec, N>;

/// Type alias for `Sender<T, tsyncp::util::codec::ProstCodec>`.
#[cfg(feature = "prost")]
pub type ProstSender<T> = Sender<T, crate::util::codec::ProstCodec>;

/// Type alias for `Receiver<T, tsyncp::util::codec::ProstCodec>`.
#[cfg(feature = "prost")]
pub type ProstReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::ProstCodec, N>;

/// Type alias for `Sender<T, tsyncp::util::codec::BincodeCodec>`.
#[cfg(feature = "bincode")]
pub type BincodeSender<T> = Sender<T, crate::util::codec::BincodeCodec>;

/// Type alias for `Receiver<T, tsyncp::util::codec::BincodeCodec>`.
#[cfg(feature = "bincode")]
pub type BincodeReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::BincodeCodec, N>;

// #[cfg(feature = "rkyv")]
// pub type RkyvSender<T, const N: usize = 0> = Sender<T, crate::util::codec::RkyvCodec>;

// #[cfg(feature = "rkyv")]
// pub type RkyvReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::RkyvCodec, N>;

/// Method to initialize [Sender]. Returns a [SenderBuilderFuture](builder::SenderBuilderFuture)
/// which can be chained with other futures to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [SenderBuilderFuture](builder::SenderBuilderFuture).
///
/// ### Example:
///
/// ```no_run
/// use tsyncp::mpsc;
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
///     let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:8000").await?;
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
pub fn sender_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::SenderBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::new_sender(dest)
}

/// Method to initialize [Receiver]. Returns a [ReceiverBuilderFuture](builder::ReceiverBuilderFuture)
/// which awaiting it builds the channel.
///
/// This future can also be chained to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [ReceiverBuilderFuture](builder::ReceiverBuilderFuture).
///
/// ```no_run
/// use tsyncp::mpsc;
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
///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000").await?;
///
///     // Accept 5 connections before receiving `Dummy` items.
///     rx.accept().num(5).await?;
///
///     // Receive some item on the channel.
///     if let Some(Ok(item)) = rx.recv().await {
///         println!("received {item:?}");
///     }
///
///     Ok(())
/// }
/// ```
pub fn receiver_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ReceiverBuilderFuture<
    A,
    T,
    E,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E>>>,
> {
    builder::new_receiver(local_addr)
}

/// Single-connection message sender that can send data to a receiver connection.
///
/// [Skip to APIs](#implementations)
///
/// ### Example:
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::mpsc;
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
///     let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:11114")
///         .retry(Duration::from_millis(500), 100)     // Retry connecting to remote address 100 times every 500 ms.
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
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Sender<T, E, S = tcp::OwnedWriteHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Sender<T, E, S> {
    fn from(c: channel::Channel<T, E, S>) -> Self {
        Self(c)
    }
}

impl<T, E, S> From<Sender<T, E, S>> for channel::Channel<T, E, S> {
    fn from(c: Sender<T, E, S>) -> Self {
        c.0
    }
}

impl<T, E, S> Sender<T, E, S> {
    /// Returns local address
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    /// Returns peer address
    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, S> Sender<T, E, S>
where
    E: EncodeMethod<T>,
    S: AsyncWrite + Unpin,
{
    /// Asynchronously send item to the peer addr
    pub async fn send(&mut self, item: T) -> Result<(), channel::errors::SinkError<E::Error>> {
        self.0.send(item).await
    }
}

/// Multi-connection message receiver that can receive data from all connected senders.
///
/// [Skip to APIs](#implementations)
///
/// ## Generic Parameters
///
/// Currently, `Receiver` has 4 generic paramters, which 2 of them are defaulted. I will go over them real quick.
///
/// First paramter, `T`, as you could guess, is the data type this receiver is receiving.
/// Technically, we could make `Receiver` recv any item without having to specify its type; however,
/// specifying the type as part of the receiver is better because it clears up what the receiver
/// expects to receive. If the receiver could receive any item type, what type should it receive?
///
/// Second parameter, `E`, is the codec type; here, you can specify anything that implements [EncodeMethod](crate::util::codec::EncodeMethod)
/// and [DecodeMethod](crate::util::codec::DecodeMethod). For Json, Protobuf, and Bincode, there
/// are already type aliases called `JsonChannel<T>`, `ProstChannel<T>`, and `BincodeChannel<T>`
/// which is a little easier than typing `Receiver<T, tsyncp::util::codec::JsonCodec>`, and so on.
///
/// Third paramter, `const N: usize = 0`, is defaulted to 0, so the user does not have to specify
/// it unless necessary. This paramter represents the length of the array used for the [StreamPool](crate::util::stream_pool::StreamPool)
/// if the user chooses to use an array instead of a vec. If `N` is `0` as defaulted, the stream
/// pool initializes with a vec; however, if it is specified to a value greater than 0, through
/// [ReceiverBuilderFuture::limit_const()](builder::ReceiverBuilderFuture::limit_const), then the stream pool
/// will be initialized as an array. For more in detail, see [StreamPool](crate::util::stream_pool::StreamPool)
/// and [ReceiverBuilderFuture::limit_const()](builder::ReceiverBuilderFuture::limit_const).
///
/// Fourth parameter, `L`, is a listener type that implements [Accept](crate::util::Accept)
/// trait. The user will not have to interact with parameter ever.
///
/// ## Gettings Started
///
/// Since initializing the receiver is demonstrated in [mpsc](crate::mpsc) module,
/// we will go over some methods you can use with Receiver.
///
/// ## Contents
/// * [Example 1](#example-1-receiving-items-as-bytes): Receiving items as bytes.
/// * [Example 2](#example-2-receiving-items-with-remote-address): Receiving items with remote address.
/// * [Example 3](#example-3-accepting-connections-while-receiving-items): Accepting connections while
/// receiving items.
/// * [Example 4](#example-4-chaining-all-futures-to-recv): Chaining all futures to `recv`.
///
/// ### Example 1: Receiving Items as Bytes
///
/// You can receive data as raw bytes before it gets decoded. (if you wanted raw json bytes, for
/// example)
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::mpsc;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     /// Receive item and return before decoding it as `Dummy` struct.
///     if let Some(Ok(bytes)) = rx.recv().as_bytes().await {
///         /// Print raw json bytes as json object.
///         println!("json object: {}", std::str::from_utf8(&bytes).unwrap());
///     }
/// #     Ok(())
/// # }
/// ```
///
/// ### Example 2: Receiving Items with Remote Address
///
/// You can receive data along with the remote address it came from.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::mpsc;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     /// Receive item and return before decoding it as `Dummy` struct.
///     if let Some(Ok((item, addr))) = rx.recv().with_addr().await {
///         println!("received {item:?} from {addr}");
///     }
/// #     Ok(())
/// # }
/// ```
///
/// You can chain `as_bytes` and `with_addr` to receive raw bytes with addr.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::mpsc;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
/// #    let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
/// #        .accept()
/// #        .num(10)
/// #        .await?;
///     if let Some(Ok((bytes, addr))) = rx.recv().as_bytes().with_addr().await {
///         println!("received {} from {addr}", std::str::from_utf8(&bytes).unwrap());
///     }
/// #     Ok(())
/// # }
/// ```
///
/// ### Example 3: Accepting Connections While Receiving Items.
///
/// You can concurrently accept connections while you are receiving items.
/// When an item is received, it will return the item along with the accept result in a tuple.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::mpsc;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     if let (Some(Ok(item)), Ok(num)) = rx.recv().accepting().await {
///         println!("{num} connections accepted");
///         println!("received {item:?}");
///     }
/// #     Ok(())
/// # }
/// ```
///
/// Above example will accept up to `10` connections since limit was set to 10. However, if no
/// limit was set during initialization, it will accept any number of incoming connections.
///
/// You can also set custom limit for accepting connections by chaining it.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::mpsc;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
/// #    let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
/// #        .accept()
/// #        .num(10)
/// #        .await?;
///     if let (Some(Ok(item)), Ok(num)) = rx.recv().accepting().num(5).await {
///         println!("{num} connections accepted");
///         println!("received {item:?}");
///     }
/// #     Ok(())
/// # }
/// ```
///
/// This will only accept 5 connections while receiving items, regardless of the limit of the
/// receiver. However, if the limit of the receiver is less than 5, it will only accept to the
/// receiver's limit.
///
/// ### Example 4: Chaining All Futures to Recv.
///
/// For our last example, we will chain all available futures to `recv` method.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::mpsc;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     if let (Some(Ok((bytes, addr))), Ok(num)) = rx
///         .recv()
///         .with_addr()                    // Receive with address.
///         .as_bytes()                     // Receive item as un-decoded bytes.
///         .accepting()                    // Accept connections while receiving.
///         .num(5)                         // Limit accepting connections to 5.
///         .filter(|a| a.port() % 2 == 1)  // Accept only odd port addresses.
///         .handle(|a| println!("{a} connected!"))
///         .await
///     {
///         assert!(num <= 5);
///         println!("{num} connections accepted");
///         println!("received {} from {addr}", std::str::from_utf8(&bytes).unwrap());
///     }
/// #     Ok(())
/// # }
/// ```
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Receiver<T, E, const N: usize = 0, L: Accept = ReadListener<TcpListener>>(
    #[pin] multi_channel::Channel<T, E, N, L>,
);

impl<T, E, const N: usize, L> AsMut<multi_channel::Channel<T, E, N, L>> for Receiver<T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut multi_channel::Channel<T, E, N, L> {
        &mut self.0
    }
}

impl<T, E, const N: usize, L> From<multi_channel::Channel<T, E, N, L>> for Receiver<T, E, N, L>
where
    L: Accept,
{
    fn from(c: multi_channel::Channel<T, E, N, L>) -> Self {
        Self(c)
    }
}

impl<T, E, const N: usize, L> From<Receiver<T, E, N, L>> for multi_channel::Channel<T, E, N, L>
where
    L: Accept,
{
    fn from(c: Receiver<T, E, N, L>) -> Self {
        c.0
    }
}

impl<T, E, const N: usize, L> Receiver<T, E, N, L>
where
    L: Accept,
{
    /// Returns the currently connected number of senders.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
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
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(rx.len(), 5);
    ///     println!("{} senders accepted!", rx.len());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns the total allowed number of senders.
    ///
    /// Trying to accept connections when full will result in an error.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
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
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(rx.len(), 10);
    ///     println!("{} connections accepted!", rx.len());
    ///
    ///     assert_eq!(rx.limit(), Some(10));
    ///     println!("channel limit = {}", rx.limit().unwrap());
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
    /// use tsyncp::mpsc;
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
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(rx.len(), 5);
    ///     println!("{} connections accepted!", rx.len());
    ///
    ///     assert!(!rx.is_full());
    ///
    ///     rx.accept().to_limit().await?;
    ///
    ///     assert_eq!(rx.limit(), Some(10));
    ///     assert_eq!(rx.len(), 10);
    ///     println!("channel limit = {}", rx.limit().unwrap());
    ///
    ///     assert!(rx.is_full());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn is_full(&self) -> bool {
        self.0.is_full()
    }

    /// Returns the local address this receiver is bound to.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
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
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000").await?;
    ///
    ///     println!("local_addr is {}", rx.local_addr());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn local_addr(&self) -> &SocketAddr {
        self.0.local_addr()
    }

    /// Returns vec of all connected senders' remote addresses.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
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
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(rx.peer_addrs().len(), 10);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.0.peer_addrs()
    }
}

impl<T, E, const N: usize, L> Receiver<T, E, N, L>
where
    L: Accept,
    L::Error: 'static,
{
    /// Returns an AcceptFuture which accepts a connection; this future can be chained to accept
    /// multiple connections, filter connections and handle accepted connections.
    ///
    /// See [AcceptFuture](crate::multi_channel::accept::AcceptFuture) for details.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
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
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000").await?;
    ///
    ///     // Accept 5 connections.
    ///     let num = rx
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

impl<T, E, const N: usize, L> Receiver<T, E, N, L>
where
    E: DecodeMethod<T>,
    L: Accept,
    L::Output: AsyncRead + Unpin,
{
    /// Returns a [RecvFuture](crate::multi_channel::recv::RecvFuture) that waits and receives an item.
    ///
    /// The future can be chained to return with the remote address, or return as raw bytes, or
    /// accept concurrently while waiting to recv.
    ///
    /// To see the chain methods, see [RecvFuture](crate::multi_channel::recv::RecvFuture).
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::mpsc;
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
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
    ///         .accept()
    ///         .num(10)
    ///         .await?;
    ///
    ///     if let Some(Ok(item)) = rx.recv().await {
    ///         println!("item: {item:?}");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn recv(&mut self) -> RecvFuture<'_, T, E, N, L> {
        self.0.recv()
    }
}
