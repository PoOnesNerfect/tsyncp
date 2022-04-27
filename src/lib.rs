// #![warn(
//     missing_debug_implementations,
//     missing_docs,
//     rust_2018_idioms,
//     unreachable_pub
// )]
// #![warn(rustdoc::missing_doc_code_examples)]

//! Synchronization primitives over TCP for message passing between services.
//!
//! ## Provided APIs
//! Currently, **tsyncp** provides 6 types of channels:
//! * [mpsc]: Multi-producer/single-consumer channel.
//! * [broadcast]: Sigle-producer/multi-consumer channel.
//! * [barrier]: Ensures multiple waiters [wait] for the barrier to [release].
//! * [spsc]: Single-producer/single-consumer channel.
//! * [channel]: Generic single-connection channel for sending/receiving data.
//! Can [split](channel::Channel::split) into `Sender` and `Receiver` pair.
//! * [multi_channel]: Generic multi-connection channel for sending/receiving data.
//! Can [split](multi_channel::Channel::split) into `Sender` and `Receiver` pair.
//!
//! **Note:** If an init method ends with `_on` (i.e. [broadcast::send_on]), it's listening on a local address;
//! if an init method ends with `_to` (i.e. [mpsc::send_to]), it's connecting to a remote address.
//!
//! See [examples] to see how they can be used in practice.
//!
//! [release]: crate::barrier::Barrier::release
//! [wait]: crate::barrier::Waiter::wait
//! [examples]: https://github.com/PoOnesNerfect/tsyncp/tree/main/examples
//!
//!
//! # Guide
//!
//! ## Contents
//! * [Features](#features)
//! * [Initializing Receiver](#initializing-receiver)
//! * [Initializing Sender](#initializing-sender)
//! * [Send and Receive Data Concurrently with Channel/MultiChannel](#send-and-receive-data-concurrently-with-channelmultichannel)
//!
//! ## Features
//!
//! By default, a few features (`json`, `protobuf`, `bincode`) are enabled for uses in
//! encoding/decoding data. (WIP - only json and protobuf are supported so far)
//!
//! If you would like to use other encoding schemes like `rkyv`, include it like this:
//!
//! ```toml
//! tsyncp = { version = "0.1", default-features = false, features = ["rkyv"] }
//! ```
//!
//! All possible features are:
//! * [full]: includes all features.
//! * [json]: serializing/deserializing data as json objects.
//! * [protobuf]: serializing/deserializing data as protobuf objects.
//!
//! **WIP** features:
//! * [bincode]: encoding/decoding data as compact bytes.
//! * [speedy]: super fast encoding/decoding of data.
//! * [rkyv]: super fast encoding/decoding of data and allows zero-copy deserialization.
//!
//! [full]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [json]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [protobuf]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [bincode]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [speedy]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [rkyv]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//!
//! ## Initializing Channels
//!
//! We'll use [mpsc] for our examples, since it's a widely known pattern.
//!
//! #### Initializing Receiver
//!
//! You can initialize the receiver with [mpsc::recv_on(_).await](mpsc::recv_on)
//!
//! ```no_run
//! use color_eyre::Result;
//! use serde::{Serialize, Deserialize};
//! use tsyncp::mpsc;
//!
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct Dummy {
//!     field1: String,
//!     field2: u64,
//!     field3: Vec<u8>,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:11114").await?;
//!
//!     // accept a new connection coming from a sender application.
//!     rx.accept().await?;
//!
//!     // after accepting connection, you can start receiving data from it.
//!     if let Some(Ok(item)) = rx.recv().await {
//!         // below is to show the type of received item.
//!         let item: Dummy = item;
//!
//!         println!("received item: {item:?}");
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! This will create a [mpsc::Receiver] that can accept connections on `"localhost:11114"`
//! and receive json data.
//!
//! `recv_on(_)` can take any parameter that implements [ToSocketAddrs](std::net::ToSocketAddrs). (i.e. `([127, 0, 0, 1], 11114)` or `"127.0.0.1:11114"`)
//!
//! [JsonReceiver](mpsc::JsonReceiver) is a type alias for `Recevier<T, JsonCodec>`.
//!
//! You can use other codecs by replacing the type specifier like `ProtobufReceiver<T>`,
//! or like `Receiver<T, ProtobufCodec>`. Other codecs are available at [util::codec].
//! Make sure that sender and receiver are using the same codec to encode/decode data.
//!
//! After initialization, you have to accept new connections to receive items. By default, it does not
//! have any connections and will return `None` when you call `recv()` on it.
//!
//!
//! #### Configuring the Builder Future
//!
//! You can append configuration methods to the `recv_on(_)` like `recv_on(_).accept(n)`.
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! use serde::{Serialize, Deserialize};
//! use tsyncp::mpsc;
//!
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #    field2: u64,
//! #    field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:11114").accept(5).await?;
//!
//! while let Some(Ok(item)) = rx.recv().await {
//!     println!("received item: {item:?}");
//! }
//!
//! # Ok(())
//! # }
//! ```
//!
//! In the above example, it will initially accept 5 connections before returning the receiver.
//! This is useful if you want to wait for connections before doing anything with it.
//!
//! You can append as many configurations as you want before calling `.await` on it.
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! use serde::{Serialize, Deserialize};
//! use tsyncp::mpsc;
//!
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #    field2: u64,
//! #    field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:11114")
//!     .limit(10)
//!     .accept_full()
//!     .set_tcp_reuseaddr(true)
//!     .set_tcp_nodelay(true)
//!     .await?;
//!
//! while let Some(Ok(item)) = rx.recv().await {
//!     println!("received item: {item:?}");
//! }
//!
//! # Ok(())
//! # }
//! ```
//!
//! Above example will set the total limit of connections to 10, and wait until it
//! accepts connections up to the limit.
//!
//! Also, it will set tcp settings for `reuseaddr` and `nodelay`.
//!
//! You can see all available configurations in [ReceiverBuilderFuture](mpsc::builder::ReceiverBuilderFuture).
//!
//!
//! #### Initializing Sender
//!
//! You can initialize [mpsc::Sender] by calling [mpsc::send_to].
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! # tokio::spawn(async move {
//! # let rx: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:11114").accept(1).await?;
//! # Ok::<_, Report>(())
//! # });
//! # tokio::time::sleep(std::time::Duration::from_millis(500)).await;
//! let mut tx: mpsc::JsonSender<Dummy> = mpsc::send_to("localhost:11114").await?;
//!
//! let dummy = Dummy {
//!     field1: String::from("hello world"),
//!     field2: 1234567,
//!     field3: vec![1, 2, 3, 4]
//! };
//!
//! tx.send(dummy).await?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! This will create an outgoing connection to `"localhost:11114"`.
//!
//! However, if the receiver is not yet initialized and accepting connections on this address, it will return
//! an error `ConnectionRefused`.
//!
//! You can append configurations to the init method to retry connections instead of failing.
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! # tokio::spawn(async move {
//! # let receiver: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:11114").accept(1).await?;
//! # Ok::<_, Report>(())
//! # });
//! let retry_interval = std::time::Duration::from_millis(500);
//! let max_retries = 100;
//!
//! let mut tx: mpsc::JsonSender<Dummy> = mpsc::send_to("localhost:11114")
//!     .retry(retry_interval, max_retries)
//!     .await?;
//!
//! let dummy = Dummy {
//!     field1: String::from("hello world"),
//!     field2: 1234567,
//!     field3: vec![1, 2, 3, 4]
//! };
//!
//! tx.send(dummy).await?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! This will retry connecting to `"localhost:11114"` 100 times with interval of 500 ms.
//! No more `ConnectionRefused`!
//!
//! You can see all available configurations in [SenderBuilderFuture](mpsc::builder::SenderBuilderFuture).
//!
//! Initializing other primitives are pretty similar. You can see how they work by going through
//! each module's documentation in the [modules](#modules) section.
//!
//! ### Send and Receive Data Concurrently with Channel/MultiChannel
//!
//! [channel] and [multi_channel] are the underlying primitives for all other primitives.
//!
//! In fact, all other primitives are just a thin wrapper around [channel::Channel] and [multi_channel::Channel].
//!
//! ##### mpsc::Sender
//! ```ignore
//! pub struct Sender(crate::channel::Channel);
//! ```
//!
//! ##### mpsc::Receiver
//! ```ignore
//! pub struct Receiver(crate::multi_channel::Channel);
//! ```
//!
//! The wrappers are created to isolate their functionalities to just sending or just receiving
//! data.
//!
//! However, at some point, you might want to send and receive using the same TCP connection.
//! In these cases, you can use [channel] and [multi_channel].
//!
//! [channel::Channel] can be used for a single connection, and [multi_channel::Channel] can be
//! used for multiple connections in a connection pool, and listen to new connections.
//!
//! We'll go over how to initialize these two primitives below.
//!
//! For [channel::Channel], you can create a new channel using either [channel::channel_on] or [channel::channel_to].
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use std::time::Duration;
//! use tsyncp::channel;
//!
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! // Creating listening channel with `channel_on`.
//! // By default, a channel waits to accept a connection before returning.
//! let mut ch1: channel::JsonChannel<Dummy> = channel::channel_on("localhost:11114").await?;
//!
//! // you can start receiving data right away.
//! if let Some(Ok(item)) = ch1.recv().await {
//!     println!("received item: {item:?}");
//!
//!     // you can also send data using the same channel.
//!     ch1.send(item).await?;
//! }
//!
//!
//! // Separate service connecting to above channel with `channel_to`.
//! let retry_interval = Duration::from_millis(500);
//! let max_retries = 100;
//!
//! let mut ch2: channel::JsonChannel<Dummy> = channel::channel_to("localhost:11114")
//!     .retry(retry_interval, max_retries)
//!     .await?;
//!
//! let dummy = Dummy {
//!     field1: String::from("hello world"),
//!     field2: 1234567,
//!     field3: vec![1, 2, 3, 4]
//! };
//!
//! ch2.send(dummy).await?;
//! if let Some(Ok(item)) = ch2.recv().await {
//!     println!("received item: {item:?}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! By default, [channel_on](channel::channel_on) waits until it accepts a single channel before
//! returning; is this behavior weird? Contact me at jack.y.l.dev@gmail.com if you want to change it, but I
//! think it makes sense since it's just waiting for a single connection.
//!
//! Anyways, [channel::Channel] can send and receive data to/from a single connection.
//!
//! All good so far, but what if you wanted to send and receive data concurrently? Then you can use [channel::Channel::split]!
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use std::time::Duration;
//! use tsyncp::{broadcast, channel, mpsc};
//!
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! // Creating listening channel with `channel_on`.
//! // By default, a channel waits to accept a connection before returning.
//! let ch: channel::JsonChannel<Dummy> = channel::channel_on("localhost:11114").await?;
//!
//! // split channel into (rx, tx) pair.
//! let (rx, tx) = ch.split();
//!
//! // Below code is just to show what type `split` returns.
//! let rx: broadcast::JsonReceiver<Dummy> = rx;
//! let tx: mpsc::JsonSender<Dummy> = tx;
//! # Ok(())
//! # }
//! ```
//!
//! As you can see, `split()` returns a pair of receiver and a sender.
//!
//! [Receiver](broadcast::Receiver) is from [broadcast], which is a single connection receiver,
//! and [Sender](mpsc::Sender) is from [mpsc], which is a single connection sender.
//!
//! Now, you can move `rx` and `tx` into separate threads or tasks, and send and receive data
//! concurrently.
//!
//! For [multi_channel::Channel], it's pretty similar.
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use std::time::Duration;
//! use tsyncp::{broadcast, mpsc, multi_channel};
//!
//! # #[derive(Debug, Clone, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! // Creating multi_channel with `channel_on`.
//! // Configure to set limit to 10 connections and wait til all 10 connections are accepted.
//! let ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
//!     .limit(10)
//!     .accept_full()
//!     .await?;
//!
//! // split channel into (rx, tx) pair.
//! let (rx, tx) = ch.split();
//!
//! // Below code is just to show what type `split` returns.
//! let rx: mpsc::JsonReceiver<Dummy> = rx;
//! let tx: broadcast::JsonSender<Dummy> = tx;
//! # Ok(())
//! # }
//! ```
//!
//! Currently, [multi_channel] can only be initialized using [multi_channel::channel_on], but I
//! might add a config in the future such as `.init_connect_to(_)` so that it can initialze outgoing connections as well.
//!
//! [multi_channel::Channel::split] returns a pair of [mpsc::Receiver] and [broadcast::Sender].
//!
//! With [mpsc::Receiver], you can receive data from all connections in the connection pool as a
//! stream, and, with [broadcast::Sender], you can broadcast data to all connections. You can also
//! move each of `tx` and `rx` into separate threads or tasks and send and receive data
//! concurrently.

pub mod barrier;
pub mod broadcast;
pub mod channel;
pub mod mpsc;
pub mod multi_channel;
pub mod spsc;
pub mod util;
