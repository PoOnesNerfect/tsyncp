//! Generic multi-connection channel for sending and receiving items.
//!
//! [Channel] is initialized by binding and listening on a local address.
//! [Channel] can be initialized with [channel_on].
//!
//! [channel_on] can take any type of parameter that implements [ToSocketAddrs](std::net::ToSocketAddrs).
//!
//! This module also contains the following modules:
//! - [builder] module: Contains [BuilderFuture](builder::BuilderFuture) for chaining and building
//! a Channel.
//! - [send] module: Contains [SendFuture](send::SendFuture) for send items.
//! - [recv] module: Contains [RecvFuture](recv::RecvFuture) for receive items,
//! - [accept] module: Contains [AcceptFuture](accept::AcceptFuture) used for accepting
//! connections, and [ChainedAcceptFuture](accept::ChainedAcceptFuture) for concurrently
//! accept connections with underlying future.
//!
//! [Skip to APIs](#modules)
//!
//! ## Contents
//! These examples will demonstrate how to initialize a Channel and how to chain configurations when initializing.
//! * [Example 1](#example-1): Basic usage.
//! * [Example 2](#example-2): Chaining futures.
//!
//! To see how you can use the Channel once initialized, see [Channel's documentation](Channel).
//!
//! ### Example 1
//!
//! Below is the basic example of initializing, accepting, receiving and sending data with [JsonChannel].
//!
//! If you want to use a channel with a different codec, just replace `JsonChannel<Dummy>`
//! with `ProstChannel<Dummy>`, for example, or `Channel<Dummy, CustomCodec>`.
//!
//! ```no_run
//! use tsyncp::multi_channel;
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
//!     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000").await?;
//!
//!     // Accept a connection.
//!     ch.accept().await?;
//!
//!     // Receive some item from all connections on the channel.
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
//! # use tsyncp::multi_channel;
//! # use serde::{Serialize, Deserialize};
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> color_eyre::Result<()> {
//!     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("127.0.0.1:8000")
//!         .limit(5)                       // Limit total number of allowed connections to 5.
//!         .set_tcp_nodelay(true)          // Set tcp option `nodelay` to `true`.
//!         .set_tcp_reuseaddr(true)        // Set tcp option `reuseaddr` to `true`.
//!         .accept()
//!         .to_limit()                     // Accept connections up to the limit before returning.
//!         .await?;
//!
//!     // Receive an item and the address where it came from.
//!     if let Some(Ok((item, addr))) = ch.recv().with_addr().await {
//!         println!("received {item:?} from {addr}");
//!
//!         // Broadcast received item to all connections except where it came from.
//!         ch.send(item).filter(|a| a != addr).await?;
//!     }
//!
//! #     Ok(())
//! # }
//! ```
//!
//! Builder future's configurable chain futures are in [builder] module.
//!
//! For extending Channel's `send` and `recv`, see documentaion for [Channel].

use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::stream_pool::StreamPool;
use crate::util::{
    listener::{ListenerWrapper, ReadListener, WriteListener},
    Accept, Split,
};
use crate::{broadcast, mpsc};
use errors::*;
use futures::{ready, Future};
use futures::{Sink, SinkExt, Stream};
use snafu::{ensure, ResultExt};
use std::fmt;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

pub mod accept;
pub mod builder;
pub mod recv;
pub mod send;

/// Type alias for `Channel<T, tsyncp::util::codec::JsonCodec>`.
#[cfg(feature = "json")]
pub type JsonChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::JsonCodec, N>;

/// Type alias for `Channel<T, tsyncp::util::codec::ProstCodec>`.
#[cfg(feature = "prost")]
pub type ProstChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::ProstCodec, N>;

/// Type alias for `Channel<T, tsyncp::util::codec::BincodeCodec>`.
#[cfg(feature = "bincode")]
pub type BincodeChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::BincodeCodec, N>;

// #[cfg(feature = "rkyv")]
// pub type RkyvChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::RkyvCodec, N>;

/// Method to initialize [Channel]. Returns a [BuilderFuture](builder::BuilderFuture)
/// which awaiting it builds the channel.
/// This future can also be chained with other futures to configure the initialization.
///
/// To see how you can chain more configurations to this method, see [BuilderFuture](builder::BuilderFuture).
///
/// ```no_run
/// use tsyncp::multi_channel;
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
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000").await?;
///
///     // Accept 5 connections before sending/receiving `Dummy` items.
///     ch.accept().num(5).await?;
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
) -> builder::BuilderFuture<A, T, E, impl Future<Output = builder::Result<Channel<T, E>>>> {
    builder::new_multi(local_addr)
}

/// Multi-connection channel that can send/receive data from all connected streams.
///
/// [Skip to APIs](#implementations)
///
/// ## Generic Parameters
///
/// Currently, `Channel` has 4 generic paramters, which 2 of them are defaulted. I will go over them real quick.
///
/// First paramter, `T`, as you could guess, is the data type this channel is sending and receiving.
/// Technically, we could make `Channel` send/recv any item without having to specify its type; however,
/// specifying the type as part of the channel is better because it clears up what the channel
/// expects to send and receive. If the sender could send any item type, there is no way for the
/// receiver to know which type to decode it to.
///
/// Second parameter, `E`, is the codec type; here, you can specify anything that implements [EncodeMethod](crate::util::codec::EncodeMethod)
/// and [DecodeMethod](crate::util::codec::DecodeMethod). For Json, Protobuf, and Bincode, there
/// are already type aliases called `JsonChannel<T>`, `ProstChannel<T>`, and `BincodeChannel<T>`
/// which is a little easier than typing `Channel<T, tsyncp::util::codec::JsonCodec>`, and so on.
///
/// Third paramter, `const N: usize = 0`, is defaulted to 0, so the user does not have to specify
/// it unless necessary. This paramter represents the length of the array used for the [StreamPool](crate::util::stream_pool::StreamPool)
/// if the user chooses to use an array instead of a vec. If `N` is `0` as defaulted, the stream
/// pool initializes with a vec; however, if it is specified to a value greater than 0, through
/// [BuilderFuture::limit_const()](builder::BuilderFuture::limit_const), then the stream pool
/// will be initialized as an array. For more in detail, see [StreamPool](crate::util::stream_pool::StreamPool)
/// and [BuilderFuture::limit_const()](builder::BuilderFuture::limit_const).
///
/// Fourth parameter, `L`, is a listener type that implements [Accept](crate::util::Accept)
/// trait. The user will not have to interact with parameter ever. This parameter is generic so
/// that the listener could be split off into [ReadListener](crate::util::listener::ReadListener)
/// and [WriteListener](crate::util::listener::WriteListener) when splitting `Channel` into
/// [Receiver](crate::mpsc::Receiver) and [Sender](crate::broadcast::Sender) pair, as will be shown
/// in the example below.
///
/// ## Gettings Started
///
/// Since initializing the channel is demonstrated in [multi_channel](crate::multi_channel) module,
/// we will go over some methods you can use with Channel.
///
/// Also, you can [split](Channel::split) a channel into [mpsc::Receiver](crate::mpsc::Receiver)
/// and [broadcast::Sender](crate::broadcast::Sender) pair to send and receive data concurrently.
/// See example for this in [Example 9](#example-9-concurrently-send-and-recv-data).
///
/// ## Contents
/// * [Example 1](#example-1-sending-items-to-specific-addresses): Sending items to specific addresses.
/// * [Example 2](#example-2-sending-items-by-filtering-addresses): Sending items by filtering addresses.
/// * [Example 3](#example-3-accepting-connections-while-sending-items): Accepting connections
/// while sending items.
/// * [Example 4](#example-4-chaining-all-futures-to-send): Chaining all futures to `send`.
/// * [Example 5](#example-5-receiving-items-as-bytes): Receiving items as bytes.
/// * [Example 6](#example-6-receiving-items-with-remote-address): Receiving items with remote address.
/// * [Example 7](#example-7-accepting-connections-while-receiving-items): Accepting connections while
/// receiving items.
/// * [Example 8](#example-8-chaining-all-futures-to-recv): Chaining all futures to `recv`.
/// * [Example 9](#example-9-concurrently-send-and-recv-data): Concurrently send and recv data.
///
/// ### Example 1: Sending Items to Specific Addresses
///
/// You can actually choose which addresses you can send the items to.
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::multi_channel;
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
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
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
///     let addrs = ch
///         .peer_addrs()
///         .into_iter()
///         .filter(|a| a.port() % 2 == 0)
///         .collect::<Vec<_>>();
///
///     // Send to only the addresses in `addrs` vec.
///     ch.send(dummy).to(&addrs).await?;
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
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
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
///     ch.send(dummy).filter(|a| a.port() % 2 == 0).await?;
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
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
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
///     let (res, accept_res) = ch
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
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
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
///     let (res, accept_res) = ch.send(dummy)
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
///
/// ### Example 5: Receiving Items as Bytes
///
/// You can receive data as raw bytes before it gets decoded. (if you wanted raw json bytes, for
/// example)
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
///         .accept()
///         .num(10)
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
/// ### Example 6: Receiving Items with Remote Address
///
/// You can receive data along with the remote address it came from.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     /// Receive item and return before decoding it as `Dummy` struct.
///     if let Some(Ok((item, addr))) = ch.recv().with_addr().await {
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
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
/// #    let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
/// #        .accept()
/// #        .num(10)
/// #        .await?;
///     if let Some(Ok((bytes, addr))) = ch.recv().as_bytes().with_addr().await {
///         println!("received {} from {addr}", std::str::from_utf8(&bytes).unwrap());
///     }
/// #     Ok(())
/// # }
/// ```
///
/// ### Example 7: Accepting Connections While Receiving Items.
///
/// You can concurrently accept connections while you are receiving items.
/// When an item is received, it will return the item along with the accept result in a tuple.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     if let (Some(Ok(item)), Ok(num)) = ch.recv().accepting().await {
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
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
/// #    let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
/// #        .accept()
/// #        .num(10)
/// #        .await?;
///     if let (Some(Ok(item)), Ok(num)) = ch.recv().accepting().num(5).await {
///         println!("{num} connections accepted");
///         println!("received {item:?}");
///     }
/// #     Ok(())
/// # }
/// ```
///
/// This will only accept 5 connections while receiving items, regardless of the limit of the
/// channel. However, if the limit of the channel is less than 5, it will only accept to the
/// channel's limit.
///
/// ### Example 8: Chaining All Futures to Recv.
///
/// For our last example, we will chain all available futures to `recv` method.
///
/// ```no_run
/// # use color_eyre::Result;
/// # use serde::{Serialize, Deserialize};
/// # use tsyncp::multi_channel;
/// # #[derive(Debug, Serialize, Deserialize)]
/// # struct Dummy {
/// #     field1: String,
/// #     field2: u64,
/// #     field3: Vec<u8>,
/// # }
/// # #[tokio::main]
/// # async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
///         .accept()
///         .num(10)
///         .await?;
///
///     if let (Some(Ok((bytes, addr))), Ok(num)) = ch
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
///
/// ### Example 9: Concurrently Send and Recv Data.
///
/// You can split the channel to send and receive items at the same time.
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::{multi_channel, mpsc, broadcast};
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
///     let ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
///         .limit(10)
///         .accept()
///         .to_limit()
///         .await?;
///
///     let (rx, tx) = ch.split();
///
///     // Below two lines are unnecessary, they are just to show the types `split` returns.
///     let mut rx: mpsc::JsonReceiver<Dummy> = rx;
///     let mut tx: broadcast::JsonSender<Dummy> = tx;
///
///     tokio::spawn(async move {
///         while let (Some(Ok(item)), _) = rx.recv().accepting().await {
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
///
/// Of course, you can still chain all the futures as we did before.
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Channel<T, E, const N: usize = 0, L = TcpListener>
where
    L: Accept,
{
    listener: L,
    local_addr: SocketAddr,
    #[pin]
    stream_pool: StreamPool<L::Output, N>,
    stream_config: L::Config,
    _phantom: PhantomData<(T, E)>,
}

impl<T, E, const N: usize, L> AsMut<Channel<T, E, N, L>> for Channel<T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        self
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    L: Accept,
{
    /// Returns the currently connected number of connections.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(ch.len(), 5);
    ///     println!("{} connections accepted!", ch.len());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn len(&self) -> usize {
        self.stream_pool.len()
    }

    /// Returns the total allowed number of connections.
    ///
    /// Trying to accept connections when full will result in an error.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(ch.len(), 10);
    ///     println!("{} connections accepted!", ch.len());
    ///
    ///     assert_eq!(ch.limit(), Some(10));
    ///     println!("channel limit = {}", ch.limit().unwrap());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn limit(&self) -> Option<usize> {
        self.stream_pool.limit()
    }

    /// Returns whether the connection limit is reached.
    ///
    /// If there is no limit, it will always return `false`.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .num(5)
    ///         .await?;
    ///
    ///     assert_eq!(ch.len(), 5);
    ///     println!("{} connections accepted!", ch.len());
    ///
    ///     assert!(!ch.is_full());
    ///
    ///     ch.accept().to_limit().await?;
    ///
    ///     assert_eq!(ch.limit(), Some(10));
    ///     assert_eq!(ch.len(), 10);
    ///     println!("channel limit = {}", ch.limit().unwrap());
    ///
    ///     assert!(ch.is_full());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn is_full(&self) -> bool {
        self.stream_pool
            .limit()
            .map(|lim| self.len() >= lim)
            .unwrap_or(false)
    }

    /// Returns the local address this channel is bound to.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000").await?;
    ///
    ///     println!("local_addr is {}", ch.local_addr());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn local_addr(&self) -> &SocketAddr {
        &self.local_addr
    }

    /// Returns vec of all connected remote addresses.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .limit(10)
    ///         .accept()
    ///         .to_limit()
    ///         .await?;
    ///
    ///     assert_eq!(ch.peer_addrs().len(), 10);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.stream_pool.addrs()
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    /// Split this Channel into [mpsc::Receiver](crate::mpsc::Receiver) and [broadcast::Sender](crate::broadcast::Sender) pair.
    ///
    /// This can be used if you want to send and receive data concurrently, or use them in different
    /// threads or tasks.
    ///
    /// Beware that, when split, if Receiver accepts connections, it will queue the write half of the
    /// connection to a write accept queue where the limit is 1024, so, if the Sender also does not call accept,
    /// older connections will be lost. Same the other way aronud.
    ///
    /// These receiver and sender can be passed into another thread or task and be used
    /// concurrently.
    ///
    /// # Example:
    ///
    /// ```no_run
    /// use tsyncp::{multi_channel, mpsc, broadcast};
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .accept()
    ///         .num(10)
    ///         .await?;
    ///
    ///     let (rx, tx) = ch.split();
    ///
    ///     // below two lines are for showing the types.
    ///     let mut rx: mpsc::JsonReceiver<Dummy> = rx;
    ///     let mut tx: broadcast::JsonSender<Dummy> = tx;
    ///
    ///     tokio::spawn(async move {
    ///         while let (Some(Ok((item, addr))), Ok(num)) = rx
    ///             .recv()
    ///             .with_addr()
    ///             .accepting()
    ///             .await
    ///         {
    ///             println!("accepted {num} connections.");
    ///             println!("received {item:?} from {addr}");
    ///         }
    ///     });
    ///
    ///     for i in 0..1_000_000 {
    ///         let dummy = Dummy {
    ///             field1: String::from("hello world"),
    ///             field2: i,
    ///             field3: Vec::new()
    ///         };
    ///
    ///         let (res, accept_res) = tx.send(dummy).accepting().await;
    ///
    ///         let num = accept_res?;
    ///         println!("accepted {num} connections.");
    ///
    ///         res?;
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn split(
        self,
    ) -> (
        mpsc::Receiver<T, E, N, ReadListener<L>>,
        broadcast::Sender<T, E, N, WriteListener<L>>,
    ) {
        Split::split(self)
    }
}

impl<T, E, const N: usize, L> Split for Channel<T, E, N, L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Left = mpsc::Receiver<T, E, N, ReadListener<L>>;
    type Right = broadcast::Sender<T, E, N, WriteListener<L>>;
    type Error = UnsplitError<<L::Output as Split>::Error>;

    fn split(self) -> (Self::Left, Self::Right) {
        let Channel {
            listener,
            local_addr,
            stream_pool,
            stream_config,
            ..
        } = self;

        let wrapper: ListenerWrapper<L> = listener.into();
        let (r_listener, w_listener) = wrapper.split();
        let (r_pool, w_pool) = stream_pool.split();

        let receiver = Channel {
            listener: r_listener,
            local_addr,
            stream_pool: r_pool,
            stream_config: stream_config.clone(),
            _phantom: PhantomData,
        };

        let sender = Channel {
            listener: w_listener,
            local_addr,
            stream_pool: w_pool,
            stream_config,
            _phantom: PhantomData,
        };

        (receiver.into(), sender.into())
    }

    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error> {
        let Channel {
            listener: l_listener,
            local_addr: l_local_addr,
            stream_pool: l_stream_pool,
            stream_config: l_stream_config,
            ..
        } = left.into();

        let Channel {
            listener: r_listener,
            local_addr: r_local_addr,
            stream_pool: r_stream_pool,
            stream_config: r_stream_config,
            ..
        } = right.into();

        ensure!(
            l_local_addr == r_local_addr,
            UnequalLocalAddrSnafu {
                l_local_addr,
                r_local_addr
            }
        );

        ensure!(l_stream_config == r_stream_config, UnequalStreamConfigSnafu);

        let listener = ListenerWrapper::<L>::unsplit(l_listener, r_listener)
            .context(ListenerUnsplitSnafu)?
            .into_inner();

        let stream_pool = StreamPool::<L::Output, N>::unsplit(l_stream_pool, r_stream_pool)
            .context(StreamPoolUnsplitSnafu)?;

        Ok(Self {
            listener,
            stream_pool,
            local_addr: l_local_addr,
            stream_config: l_stream_config,
            _phantom: PhantomData,
        })
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
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
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000").await?;
    ///
    ///     // Accept 5 connections.
    ///     let num = ch
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
    ) -> accept::AcceptFuture<'_, T, E, N, L, impl FnMut(SocketAddr), impl FnMut(SocketAddr) -> bool>
    {
        accept::AcceptFuture::new(self, |_| {}, |_| true)
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    /// Returns a [RecvFuture](recv::RecvFuture) that waits and receives an item.
    ///
    /// The future can be chained to return with the remote address, or return as raw bytes, or
    /// accept concurrently while waiting to recv.
    ///
    /// To see the chain methods, see [RecvFuture](recv::RecvFuture).
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
    ///         .accept()
    ///         .num(10)
    ///         .await?;
    ///
    ///     if let Some(Ok(item)) = ch.recv().await {
    ///         println!("item: {item:?}");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn recv(&mut self) -> recv::RecvFuture<'_, T, E, N, L> {
        recv::RecvFuture::new(self)
    }
}

impl<T, E: DecodeMethod<T>, const N: usize, L: Accept> Stream for Channel<T, E, N, L>
where
    L::Output: AsyncRead + Unpin,
{
    type Item = Result<T, StreamError<E::Error>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let (frame, addr) = match ready!(self.project().stream_pool.poll_next(cx)) {
            Some((Ok(frame), addr)) => (frame, addr),
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(StreamSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame).context(ItemDecodeSnafu { addr });

        Poll::Ready(Some(decoded))
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    E: EncodeMethod<T>,
    L: Accept,
    L::Output: AsyncWrite + Unpin,
{
    /// Returns [SendFuture](send::SendFuture) that sends and flushes an item to all connections.
    ///
    /// The future can be chained to send item to only specific addresses, filter addresses, and
    /// accept connections concurrently while waiting to send and flush items.
    ///
    /// To see the chain methods, see [SendFuture](send::SendFuture).
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::multi_channel;
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
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
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
    ///     ch.send(dummy).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn send(&mut self, item: T) -> send::SendFuture<'_, T, E, N, L> {
        send::SendFuture::new(self, item)
    }
}

impl<T, E, const N: usize, L> Sink<T> for Channel<T, E, N, L>
where
    L: Accept,
    E: EncodeMethod<T>,
    L::Output: AsyncWrite + Unpin,
{
    type Error = SinkError<E::Error>;

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu {
            addr: *self.local_addr(),
        })?;

        self.stream_pool
            .start_send_unpin(encoded)
            .context(SinkErrorsSnafu {
                addr: *self.local_addr(),
            })?;

        Ok(())
    }

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.stream_pool.poll_ready_unpin(cx)).context(SinkErrorsSnafu {
            addr: *self.local_addr(),
        });

        Poll::Ready(res)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.stream_pool.poll_flush_unpin(cx)).context(SinkErrorsSnafu {
            addr: *self.local_addr(),
        });

        Poll::Ready(res)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.stream_pool.poll_close_unpin(cx)).context(SinkErrorsSnafu {
            addr: *self.local_addr(),
        });

        Poll::Ready(res)
    }
}

#[allow(missing_docs)]
pub mod errors {
    use crate::util::{listener, stream_pool};
    use snafu::{Backtrace, Snafu};
    use std::io::ErrorKind;
    use std::net::SocketAddr;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SinkError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[Encode Error] Failed to encode item on {addr}"))]
        ItemEncode {
            addr: SocketAddr,
            source: E,
            backtrace: Backtrace,
        },
        #[snafu(display("[Sink Error] Failed to send item on {addr}"))]
        SinkErrors {
            addr: SocketAddr,
            source: stream_pool::errors::SinkErrors,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug)]
    pub struct SinkErrorIterator<'a, I>
    where
        I: Iterator<Item = &'a stream_pool::errors::PollError>,
    {
        iter: Option<I>,
    }

    impl<'a, I> Iterator for SinkErrorIterator<'a, I>
    where
        I: Iterator<Item = &'a stream_pool::errors::PollError>,
    {
        type Item = &'a stream_pool::errors::PollError;

        fn next(&mut self) -> Option<Self::Item> {
            if let Some(iter) = &mut self.iter {
                iter.next()
            } else {
                None
            }
        }
    }

    #[derive(Debug)]
    pub struct SinkErrorIntoIterator<I>
    where
        I: Iterator<Item = stream_pool::errors::PollError>,
    {
        iter: Option<I>,
    }

    impl<I> Iterator for SinkErrorIntoIterator<I>
    where
        I: Iterator<Item = stream_pool::errors::PollError>,
    {
        type Item = stream_pool::errors::PollError;

        fn next(&mut self) -> Option<Self::Item> {
            if let Some(iter) = &mut self.iter {
                iter.next()
            } else {
                None
            }
        }
    }

    impl<E> SinkError<E>
    where
        E: 'static + snafu::Error,
    {
        pub fn local_addr(&self) -> &SocketAddr {
            match self {
                Self::SinkErrors { addr, .. } => addr,
                Self::ItemEncode { addr, .. } => addr,
            }
        }

        pub fn peer_addrs(&self) -> Vec<SocketAddr> {
            match self {
                Self::SinkErrors { source, .. } => source.peer_addrs(),
                Self::ItemEncode { .. } => Vec::new(),
            }
        }

        pub fn is_encode_error(&self) -> bool {
            matches!(self, Self::ItemEncode { .. })
        }

        pub fn is_sink_error(&self) -> bool {
            !self.is_encode_error()
        }

        pub fn as_sink_errors(
            &self,
        ) -> SinkErrorIterator<'_, impl Iterator<Item = &stream_pool::errors::PollError>> {
            let iter = if let Self::SinkErrors { source, .. } = self {
                Some(source.iter())
            } else {
                None
            };

            SinkErrorIterator { iter }
        }

        pub fn into_sink_errors(
            self,
        ) -> SinkErrorIntoIterator<impl Iterator<Item = stream_pool::errors::PollError>> {
            let iter = if let Self::SinkErrors { source, .. } = self {
                Some(source.into_iter())
            } else {
                None
            };

            SinkErrorIntoIterator { iter }
        }

        pub fn as_io_errors(&self) -> impl Iterator<Item = &std::io::Error> {
            self.as_sink_errors().filter_map(|e| e.as_io())
        }

        pub fn into_io_errors(self) -> impl Iterator<Item = std::io::Error> {
            self.into_sink_errors().filter_map(|e| e.into_io())
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum StreamError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[Decode Error] Failed to decode frame of item on {addr}"))]
        ItemDecode {
            addr: SocketAddr,
            source: E,
            backtrace: Backtrace,
        },
        #[snafu(display("[Stream Error] Failed receive item on {addr}"))]
        StreamError {
            addr: SocketAddr,
            source: stream_pool::errors::PollError,
            backtrace: Backtrace,
        },
    }

    impl<E> StreamError<E>
    where
        E: snafu::Error,
    {
        pub fn local_addr(&self) -> &SocketAddr {
            match self {
                Self::StreamError { addr, .. } => addr,
                Self::ItemDecode { addr, .. } => addr,
            }
        }

        pub fn peer_addr(&self) -> Option<SocketAddr> {
            match self {
                Self::ItemDecode { .. } => None,
                Self::StreamError { source, .. } => Some(*source.peer_addr()),
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
                Self::StreamError { source, .. } => source.as_io(),
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
        #[snafu(display("[Unsplit Error] Underlying channels' local addrs are different: {l_local_addr:?} != {r_local_addr:?}"))]
        UnequalLocalAddr {
            l_local_addr: SocketAddr,
            r_local_addr: SocketAddr,
        },
        #[snafu(display("[Unsplit Error] Underlying channels' stream configs are different"))]
        UnequalStreamConfig,
        #[snafu(display("[Unsplit Error] Failed to split underlying listener"))]
        ListenerUnsplitError {
            source: listener::errors::UnsplitError,
            backtrace: Backtrace,
        },
        #[snafu(display("[Unsplit Error] Failed to split underlying stream pool"))]
        StreamPoolUnsplitError {
            source: stream_pool::errors::UnsplitError<E>,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum AcceptError<E: 'static + snafu::Error> {
        #[snafu(display("[Accept Error] Underlying stream pool failed to accept"))]
        StreamPoolAcceptError { source: E, backtrace: Backtrace },
        #[snafu(display("[Accept Error] Failed to push accepted stream to stream pool"))]
        PushStream {
            source: stream_pool::errors::StreamPoolError,
            backtrace: Backtrace,
        },
    }
}
