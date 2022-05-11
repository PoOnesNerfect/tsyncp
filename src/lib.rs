#![warn(missing_debug_implementations)]
#![warn(rust_2018_idioms)]
#![warn(unreachable_pub)]
#![warn(rustdoc::missing_doc_code_examples)]

//! Synchronization primitives over TCP for message passing between services.
//!
//! ## Provided APIs
//! Currently, **tsyncp** provides 6 types of channels:
//! * [mpsc]: Multi-producer/single-consumer channel.
//! * [broadcast]: Sigle-producer/multi-consumer channel.
//! * [barrier]: Multiple waiters [wait] for a barrier to [release].
//! * [spsc]: Single-producer/single-consumer channel.
//! * [channel]: Generic single-connection channel for sending/receiving data.
//! Can [split](channel::Channel::split) into [Sender](mpsc::Sender) and [Receiver](broadcast::Receiver) pair.
//! * [multi_channel]: Generic multi-connection channel for sending/receiving data.
//! Can [split](multi_channel::Channel::split) into [Sender](broadcast::Sender) and [Receiver](mpsc::Receiver) pair.
//!
//! See [Guide](#guide) below or repository's [examples] to see how they can be used in practice.
//!
//! [release]: crate::barrier::Barrier::release
//! [wait]: crate::barrier::Waiter::wait
//! [examples]: https://github.com/PoOnesNerfect/tsyncp/tree/main/examples
//!
//!
//! [Skip to APIs](#modules)
//!
//! # Guide
//!
//! ## Contents
//! * [Initializing Receiver](#initializing-receiver)
//! * [Initializing Sender](#initializing-sender)
//! * [Features](#features)
//!
//! ## Advanced Contents
//! * [Chaining the Builder Future](#chaining-the-builder-future)
//! * [Chaining the Recv Future](#chaining-the-recv-future)
//! * [Chaining the Send Future](#chaining-the-send-future)
//! * [Chaining the Accept Future](#chaining-the-accept-future)
//! * [Send/Receive on the Same Connection with Channel/MultiChannel](#sendreceive-on-the-same-connection-with-channelmultichannel)
//! * [Using Custom Encoding/Decoding Methods](#using-custom-encodingdecoding-methods)
//! * [Error handling](#error-handling)
//!
//! ## Initializing Receiver
//!
//! We'll use [mpsc] for our examples, since it's a widely known pattern.
//!
//! You can initialize the receiver with [mpsc::receiver_on(_).await](mpsc::receiver_on).
//! Since the receiver receives data from multiple senders, it will listen on a local address for
//! incoming connections from senders.
//!
//! ```no_run
//! use color_eyre::Result;
//! use serde::{Serialize, Deserialize};
//! use tsyncp::mpsc;
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct Dummy {
//!     field1: String,
//!     field2: u64,
//!     field3: Vec<u8>,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114").await?;
//!
//!     // accept a new connection coming from a sender application.
//!     rx.accept().await?;
//!
//!     // after accepting connection, you can start receiving data from the receiver.
//!     if let Some(Ok(item)) = rx.recv().await {
//!         // below line is to show the type of received item.
//!         let item: Dummy = item;
//!
//!         println!("received item: {item:?}");
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! Above lines will create a receiver, accept a connection, then receive an item on the receiver.
//!
//! `receiver_on(_)` can take any parameter that implements [ToSocketAddrs](std::net::ToSocketAddrs). (i.e. `"127.0.0.1:11114"` or `("localhost", 11114)`)
//!
//! After initialization, you have to accept connections to receive items. By default, it does not
//! accept any connections and will return `None` when you call `recv()` on it.
//!
//! [JsonReceiver](mpsc::JsonReceiver) is a type alias for `Recevier<T, JsonCodec>`. `T` can be any
//! type that implements `serde::Serialize` and `serde::Deserialize`.
//!
//! You can use other codecs by replacing the type specifier as `ProstReceiver<T>`, for example:
//! ```no_run
//! # use color_eyre::Result;
//! # use tsyncp::mpsc;
//! use prost::Message;
//!
//! #[derive(Message)]
//! struct Dummy {
//!     #[prost(string, tag = "1")]
//!     field1: String,
//!     #[prost(uint64, tag = "2")]
//!     field2: u64,
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::ProstReceiver<Dummy> = mpsc::receiver_on("localhost:11114").await?;
//! #     Ok(())
//! # }
//! ```
//! or as `Receiver<T, ProstCodec>`:
//! ```no_run
//! # use color_eyre::Result;
//! # use tsyncp::mpsc;
//! # use prost::Message;
//! # #[derive(Message)]
//! # struct Dummy {
//! #     #[prost(string, tag = "1")]
//! #     field1: String,
//! #     #[prost(uint64, tag = "2")]
//! #     field2: u64,
//! # }
//! #
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! use tsyncp::util::codec::ProstCodec;
//!
//! let mut rx: mpsc::Receiver<Dummy, ProstCodec> = mpsc::receiver_on("localhost:11114").await?;
//! #     Ok(())
//! # }
//! ```
//!
//! Other codecs are available at [util::codec].
//!
//! Make sure that sender and receiver are using the same codec to encode/decode data.
//!
//! See [Chaining the Builder Future](#chaining-the-builder-future) and [Chaining the Recv Future](#chaining-the-recv-future) to extend the APIs.
//!
//! **Note:** If an init method ends with `_on` (i.e. [mpsc::receiver_on]), it's listening on a local address;
//! if an init method ends with `_to` (i.e. [broadcast::receiver_to]), it's connecting to a remote address.
//!
//! ## Initializing Sender
//!
//! You can initialize [mpsc::Sender] by calling [mpsc::sender_to].
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! # tokio::time::sleep(std::time::Duration::from_millis(500)).await;
//! let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:11114").await?;
//!
//! let dummy = Dummy {
//!     field1: String::from("hello world"),
//!     field2: 1234567,
//!     field3: vec![1, 2, 3, 4]
//! };
//!
//! tx.send(dummy).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Above lines will create an outgoing connection to `"localhost:11114"`, and send a dummy data to it.
//!
//! If the receiver is not yet initialized and accepting connections on this address, it will return
//! an error `ConnectionRefused`.
//!
//! To see how to retry connection and extend the sender APIs, see
//! [Chaining the Builder Future](#chaining-the-builder-future) and [Chaining the Send Future](#chaining-the-send-future).
//!
//! **Note:** If an init method ends with `_on` (i.e. [broadcast::sender_on]), it's listening on a local address;
//! if an init method ends with `_to` (i.e. [mpsc::sender_to]), it's connecting to a remote address.
//!
//! ## Features
//!
//! By default, `full` feature (`json`, `prost`, and `bincode`) is enabled for uses in
//! encoding/decoding data.
//!
//! If you only need one encoding/decoding scheme, disable `default-features`, and
//! include only the ones you want.
//!
//! ```toml
//! tsyncp = { version = "0.3", default-features = false, features = ["bincode"] }
//! ```
//!
//! All possible features are:
//! * [full]: includes all features. (json, prost, bincode)
//! * [json]: serializing/deserializing data as json objects.
//! * [prost]: serializing/deserializing data as protobuf objects.
//! * [bincode]: encoding/decoding data as compact bytes.
//!
//! **WIP** features:
//! * `speedy`: super fast encoding/decoding of data.
//! * `rkyv`: super fast encoding/decoding of data and allows zero-copy deserialization.
//! * Any other ones I should support?
//!
//! [full]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L21
//! [json]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L22
//! [prost]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L23
//! [bincode]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L24
//!
//! ## Chaining the Builder Future
//!
//! You can chain various configurations to the builder methods `receiver_on` and `sender_to`.
//!
//! ### Chaining the Receiver Builder Future
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #    field2: u64,
//! #    field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
//!     .limit(10)                              // limit allowed connections to 10.
//!     .set_tcp_reuseaddr(true)                // set tcp config reuseaddr to `true`.
//!     .accept()                               // accept connection. (default: 1)
//!     .to_limit()                             // accept until limit is reached. (10)
//!     .handle(|a| println!("{a} connected!")) // print address when a connection is accepted.
//!     .await?;
//!
//! // At this point, the receiver has 10 connections in the connection pool,
//! // which all have `reuseaddr` as `true`.
//! while let Some(Ok(item)) = rx.recv().await {
//!     println!("received item: {item:?}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Above code will create a receiver that limits the number of allowed connections to `10`,
//! and set the TCP setting `reuseaddr` to `true`. Also, it will print `localhost:67110 connected!`
//! whenever a new connection is accepted in this future.
//!
//! You can chain as many configurations to the method as you want. However, `accept` can only be
//! appended as the last chain method.
//! You can chain `accept` method with sub-configurations such as `num`, `to_limit`, `filter`, `handle` in any order.
//!
//! Since `mpsc::Receiver` is a multi-connection channel, you can chain `accept()` to also
//! accept connections before returning the channel.
//! However, [broadcast::Receiver] is a single-connection channel, and thus will not have `accept` chain method.
//! See builder configurations for `broadcast::Receiver` at [ReceiverBuilderFuture](broadcast::builder::ReceiverBuilderFuture).
//!
//! All the available configurations are:
//! - [limit(n: usize)](mpsc::builder::ReceiverBuilderFuture::limit): limits the total number
//! of allowed connections to `n`.
//! - [limit_const\<const N: usize\>()](mpsc::builder::ReceiverBuilderFuture::limit_const): limits the total number
//! of allowed connections to `N`; using a const value makes the [StreamPool](util::stream_pool::StreamPool)
//! use an array of const size instead of a vec.
//! - [set_tcp_reuseaddr(b: bool)](mpsc::builder::ReceiverBuilderFuture::set_tcp_reuseaddr): sets TCP
//! setting `reuseaddr` to `b`.
//! - [set_tcp_reuseport(b: bool)](mpsc::builder::ReceiverBuilderFuture::set_tcp_reuseport): sets TCP
//! setting `reuseport` to `b`.
//! - [set_tcp_linger(dur: Option\<Duration\>)](mpsc::builder::ReceiverBuilderFuture::set_tcp_linger): sets TCP
//! setting `linger` to `dur`.
//! - [set_tcp_nodelay(b: bool)](mpsc::builder::ReceiverBuilderFuture::set_tcp_nodelay): sets TCP
//! setting `nodelay` to `b`.
//! - [set_tcp_ttl(dur: u32)](mpsc::builder::ReceiverBuilderFuture::set_tcp_ttl): sets TCP
//! setting `ttl` to `dur`.
//! - [set_tcp_recv_buffer_size(n: u32)](mpsc::builder::ReceiverBuilderFuture::set_tcp_recv_buffer_size): sets TCP
//! setting `recv_buffer_size` to `n`.
//! - [set_tcp_send_buffer_size(n: u32)](mpsc::builder::ReceiverBuilderFuture::set_tcp_send_buffer_size): sets TCP
//! setting `send_buffer_size` to `n`.
//! - [accept()](mpsc::builder::ReceiverBuilderFuture::accept): accept a connection before
//! returning `Receiver`. There are sub-configurations for `accept` to chain such as `num`,
//! `to_limit`, `filter` and `handle`.
//! - [accept().num(n: usize)](multi_channel::builder::AcceptBuilderFuture::num): accept `n` connections before
//! returning `Receiver`. Use this or `to_limit` to accept multiple connections.
//! - [accept().to_limit()](multi_channel::builder::AcceptBuilderFuture::to_limit): accept connections
//! until the limit is reached. If no limit is set for the builder, it will only accept one
//! connection. Use this or `num` to accept multiple connections.
//! - [accept().filter(|a: SocketAddr| -> bool)](multi_channel::builder::AcceptBuilderFuture::filter): filter
//! the accepted connections. If the `filter(address)` returns `false`, the connection will be ignored.
//! You can pass references to the closure, which is useful if you want to do something with outer variables.
//! - [accept().handle(|a: SocketAddr| -> ())](multi_channel::builder::AcceptBuilderFuture::handle): after a
//! connection is included to the pool, react to it immediately with the remote address.
//! You can pass references to the closure, which is useful if you want to do something with outer variables.
//!
//! ### Chaining the Sender Builder Future
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #    field2: u64,
//! #    field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! use std::time::Duration;
//!
//! let retry_interval = Duration::from_millis(500);
//! let max_retries = 100;
//!
//! let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:11114")
//!     .retry(retry_interval, max_retries) // retry connecting 100 times every 500 ms.
//!     .set_tcp_reuseaddr(true)            // set tcp config reuseaddr to `true`.
//!     .await?;
//!
//! # let dummy = Dummy {
//! #     field1: String::from("hello world"),
//! #     field2: 1234567,
//! #     field3: vec![1, 2, 3, 4]
//! # };
//! // send some item.
//! tx.send(dummy).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Above code will try to connect to the receiver on `"localhost:11114"`, and retry 100 times
//! every 500 ms when it fails. This is useful if you are not sure whether the receiver application
//! is running and accepting connections.
//!
//! You can chain as many configurations to the method as you want.
//!
//! Since `mpsc::Sender` is a single-connection channel, there is no chain method `accept`.
//! However, [broadcast::Sender] is a multi-connection channel, thus can be chained with
//! [accept()](broadcast::Sender::accept).
//! See builder configurations for [broadcast::Sender] at [SenderBuilderFuture](broadcast::builder::SenderBuilderFuture).
//!
//! All the available configurations are:
//! - [retry(interval: Duration, max_retries: usize)](mpsc::builder::SenderBuilderFuture::retry): retry connecting to
//! `max_retries` times with the interval of `interval`.
//! - [set_tcp_reuseaddr(b: bool)](mpsc::builder::SenderBuilderFuture::set_tcp_reuseaddr): sets TCP
//! setting `reuseaddr` to `b`.
//! - [set_tcp_reuseport(b: bool)](mpsc::builder::SenderBuilderFuture::set_tcp_reuseport): sets TCP
//! setting `reuseport` to `b`.
//! - [set_tcp_linger(dur: Option\<Duration\>)](mpsc::builder::SenderBuilderFuture::set_tcp_linger): sets TCP
//! setting `linger` to `dur`.
//! - [set_tcp_nodelay(b: bool)](mpsc::builder::SenderBuilderFuture::set_tcp_nodelay): sets TCP
//! setting `nodelay` to `b`.
//! - [set_tcp_ttl(dur: u32)](mpsc::builder::SenderBuilderFuture::set_tcp_ttl): sets TCP
//! setting `ttl` to `dur`.
//! - [set_tcp_recv_buffer_size(n: u32)](mpsc::builder::SenderBuilderFuture::set_tcp_recv_buffer_size): sets TCP
//! setting `recv_buffer_size` to `n`.
//! - [set_tcp_send_buffer_size(n: u32)](mpsc::builder::SenderBuilderFuture::set_tcp_send_buffer_size): sets TCP
//! setting `send_buffer_size` to `n`.
//!
//! ## Chaining the Recv Future
//!
//! You can chain configurations to the [recv()](mpsc::Receiver::recv) method.
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #    field2: u64,
//! #    field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
//!     .accept()                       // accept connection. (default: 1)
//!     .await?;
//!
//! while let Some(Ok((bytes, addr))) = rx.recv().as_bytes().with_addr().await {
//!     println!("received json object {} from {addr}", std::str::from_utf8(&bytes).unwrap());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [as_bytes()](multi_channel::recv::RecvFuture::as_bytes) receives items as `BytesMut` before being decoded.
//!
//! [with_addr()](multi_channel::recv::RecvFuture::with_addr) returns the remote address the data was sent from.
//!
//! You can chain as many configurations to the method as you want. However,
//! [accepting()](multi_channel::recv::RecvFuture::accepting) can only be appended as the last chain method.
//! You can chain `accepting` method with sub-configurations such as `num`, `to_limit`, `filter`, `handle` in any order.
//!
//! Since `mpsc::Receiver` is a multi-connection channel, you can chain [accepting()](multi_channel::recv::RecvFuture::accepting)
//! to concurrently accept connections while waiting to receive data.
//! However, [broadcast::Receiver] is a single-connection channel, thus cannot be chained with
//! `accepting` chain method. See available chain methods for [broadcast::Receiver] at [RecvFuture](channel::recv::RecvFuture).
//!
//! All the available configurations are:
//! - [as_bytes()](multi_channel::recv::RecvFuture::as_bytes): returns bytes before decoded instead
//! of decoded item `BytesMut`.
//! - [with_addr()](multi_channel::recv::RecvFuture::with_addr): returns remote address along with
//! the item as a result of a tuple `Result<(T, SocketAddr)>`.
//! - [accepting()](multi_channel::recv::RecvFuture::accepting): while waiting to receive data,
//! concurrently poll accept to try accepting connections. By default, `accepting` accepts
//! connections up to the limit if limit exists or unlimited if limit does not exist. Chaining this
//! method returns a tuple of results `(Result<T>, Result<usize>)`, where `usize` is the number of
//! accepted connections.
//! - [accepting().num(n: usize)](multi_channel::accept::ChainedAcceptFuture::num): sets the
//! concurrently accepting number of connections to `n`. If `n` is greater than the limit, it will
//! only accept up to the limit of the channel.
//! - [accepting().to_limit()](multi_channel::accept::ChainedAcceptFuture::to_limit): only accept
//! connections up to the limit of the channel.
//! - [accepting().filter(|a: SocketAddr| -> bool)](multi_channel::accept::ChainedAcceptFuture::filter):
//! while accepting, only accept addresses where the filter closure returns `true`. You can pass
//! references to the closure, which is useful if you want to do something with outer variables.
//! - [accepting().handle(|a: SocketAddr| -> ())](multi_channel::accept::ChainedAcceptFuture::handle): after a
//! connection is included to the pool, react to it immediately with the remote address. You can
//! pass references to the closure, which is useful if you want to do something with outer variables.
//!
//! ## Chaining the Send Future
//!
//! Since [mpsc::Sender] is a single-connection channel, it does not have any chain methods.
//!
//! However, [broadcast::Sender] is a multi-connection channel, and thus can be chained with methods.
//! Therefore, we'll go over the chain methods for [broadcast::Sender::send].
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! use tsyncp::broadcast;
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #    field2: u64,
//! #    field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut tx: broadcast::JsonSender<Dummy> = broadcast::sender_on("localhost:11114")
//!     .accept()
//!     .await?;
//! # let dummy = Dummy {
//! #     field1: String::from("hello world"),
//! #     field2: 1234567,
//! #     field3: vec![1, 2, 3, 4]
//! # };
//!
//! let (send_res, accept_res) = tx.send(dummy)
//! .filter(|a| a.port() % 2 == 0)              // send only to even port addresses.
//! .accepting()                                // concurrently accept connections.
//! .handle(|a| println!("{a} connected!"))     // print when new connection is accepted.
//! .await;
//!
//! send_res?;
//! accept_res?;
//! # Ok(())
//! # }
//! ```
//!
//! [filter(|a: SocketAddr| -> bool)](multi_channel::send::SendFuture::filter) sends data only to the addresses where the
//! filter closure returns `true`.
//!
//! [accepting()](multi_channel::send::SendFuture::accepting) concurrently accepts connections while
//! waiting to send data. By default, `accepting` accepts connections up to the limit if limit
//! exists or unlimited if limit does not exist.
//!
//! You can chain as many configurations to the method as you want. However, `accepting` can only be
//! appended as the last chain method. You can chain `accepting` method with sub-configurations
//! such as `num`, `to_limit`, `filter`, `handle` in any order.
//!
//! All the available configurations are:
//! - [to(addrs: &\[SocketAddr\])](multi_channel::send::SendFuture::to): only send to provided
//! slice of addresses.
//! - [filter(|a: SocketAddr| -> bool)](multi_channel::send::SendFuture::filter): only send to addresses
//! where the filter closure returns `true`.
//! - [accepting()](multi_channel::send::SendFuture::accepting): while waiting to send data,
//! concurrently poll accept to try accepting connections. By default, `accepting` accepts
//! connections up to the limit if limit exists or unlimited if limit does not exist.
//! - [accepting().num(n: usize)](multi_channel::accept::ChainedAcceptFuture::num): sets the
//! concurrently accepting number of connections to `n`. If `n` is greater than the limit, it will
//! only accept up to the limit of the channel.
//! - [accepting().to_limit()](multi_channel::accept::ChainedAcceptFuture::to_limit): only accept
//! connections up to the limit of the channel.
//! - [accepting().filter(|a: SocketAddr| -> bool)](multi_channel::accept::ChainedAcceptFuture::filter):
//! while accepting, only accept addresses where the filter closure returns `true`. You can pass
//! references to the closure, which is useful if you want to do something with outer variables.
//! - [accepting().handle(|a: SocketAddr| -> ())](multi_channel::accept::ChainedAcceptFuture::handle): after a
//! connection is included to the pool, react to it immediately with the remote address. You can pass
//! references to the closure, which is useful if you want to do something with outer variables.
//!
//! ## Chaining the Accept Future
//!
//! Instead of chaining methods to [recv()](multi_channel::Channel::recv)
//! like [rx.recv().accepting()](multi_channel::recv::RecvFuture::accepting),
//! you can directly call [accept()](multi_channel::Channel::accept) on a channel.
//!
//! ```no_run
//! # use color_eyre::Result;
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! #
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! #
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114").await?;
//!
//! rx.accept().await?;             // accept a single channel.
//! #     Ok(())
//! # }
//! ```
//!
//! Above code will create a receiver, then accept a single connection.
//!
//! You can chain methods to this method to extend it.
//!
//! ```no_run
//! # use color_eyre::Result;
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! #
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! #
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
//!     .limit(10)                          // set limit of receiver to 10.
//!     .await?;
//!
//! rx
//! .accept()
//! .to_limit()                             // accept connections up to the limit. (10)
//! .filter(|a| a.port() % 2 == 0)          // only accept connections with even port.
//! .handle(|a| println!("{a} connected"))  // print accepted connections.
//! .await?;
//! #     Ok(())
//! # }
//! ```
//!
//! [to_limit()](multi_channel::accept::AcceptFuture::to_limit) accepts connections up to the limit.
//! If no limit is set, it defaults to `1`.
//!
//! [filter(|a: SocketAddr| -> bool)](multi_channel::accept::AcceptFuture::filter) only accepts connections where the filter
//! closure returns `true`. You can pass references to the closure, which is
//! useful if you want to do something with outer variables.
//!
//! [handle(|a: SocketAddr| -> ())](multi_channel::accept::AcceptFuture::handle) lets the user
//! react to the newly accepted connection. You can pass references to the closure, which is
//! useful if you want to do something with outer variables.
//!
//! Furthermore, there is also [with_future(async { .. })](multi_channel::accept::AcceptFuture::with_future),
//! which takes a future, then tries accepting connections until this given future finishes. When future
//! finishes, it returns a tuple of accept result and the future's output `(Result<usize, AcceptingError>, Output)`,
//! where usize is the number of connections accepted.
//!
//! ```no_run
//! # use color_eyre::Result;
//! # use serde::{Serialize, Deserialize};
//! # use tsyncp::mpsc;
//! #
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! #
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
//!     .limit(10)                          // set limit of receiver to 10.
//!     .await?;
//!
//! let x = String::from("future ending");
//!
//! let (accept_res, _) = rx
//! .accept()
//! .handle(|a| println!("{a} connected"))  // print accepted connections.
//! .with_future(async {
//!     use std::time::Duration;
//!     tokio::time::sleep(Duration::from_millis(10_000)).await;
//!     println!("{}", &x);
//! })
//! .await;
//!
//! if let Ok(num) = accept_res {
//!     println!("accepted {num} connections");
//! }
//! #     Ok(())
//! # }
//! ```
//!
//! Above code will execute the future given to `with_future(..)`, while accepting connections.
//! Since the future sleeps for 10 seconds, it will accept incoming connections for 10 seconds.
//!
//! You can also pass references to the async clause, which is handy if you're doing something with
//! outer variables.
//!
//! ## Send/Receive on the Same Connection with Channel/MultiChannel
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
//! ##### broadcast::Sender
//! ```ignore
//! pub struct Sender(crate::multi_channel::Channel);
//! ```
//!
//! ##### broadcast::Receiver
//! ```ignore
//! pub struct Receiver(crate::channel::Channel);
//! ```
//!
//! The wrappers are created to isolate their functionalities to just sending or just receiving
//! data.
//!
//! However, at some point, you may want to send and receive using the same TCP connection.
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
//! # #[derive(Debug, Serialize, Deserialize)]
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
//! // Different service connecting to above channel with `channel_to`.
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
//! # #[derive(Debug, Serialize, Deserialize)]
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
//! concurrently on the same TCP connection.
//!
//! For [multi_channel::Channel], it's pretty similar.
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize};
//! # use std::time::Duration;
//! use tsyncp::{broadcast, mpsc, multi_channel};
//!
//! # #[derive(Debug, Serialize, Deserialize)]
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
//!     .accept()
//!     .to_limit()
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
//! might add a config in the future such as `.init_connect_to(_)` so that it can initialize outgoing connections as well.
//!
//! [multi_channel::Channel::split] returns a pair of [mpsc::Receiver] and [broadcast::Sender].
//!
//! With [mpsc::Receiver], you can receive data from all connections in the connection pool as a
//! stream, and, with [broadcast::Sender], you can broadcast data to all connections. You can also
//! move each of `tx` and `rx` into separate threads or tasks and send and receive data
//! concurrently.
//!
//! ### Using Custom Encoding/Decoding Methods
//!
//! Tsyncp uses custom traits: [EncodeMethod](util::codec::EncodeMethod) and [DecodeMethod](util::codec::DecodeMethod)
//! for encoding/decoding data.
//!
//! This is because there is no united way to implement serializing/deserializing; serde comes
//! close, but not quite.
//!
//! Therefore, these custom traits provide a very simple way for the users to implement
//! serializing/deserializing data.
//!
//! The traits themselves are extremely simple:
//!
//! ```ignore
//! pub trait EncodeMethod<T> {
//!     type Error: 'static + snafu::Error;
//!
//!     fn encode(data: &T) -> Result<Bytes, Self::Error>;
//! }
//!
//! pub trait DecodeMethod<T> {
//!     type Error: 'static + snafu::Error;
//!
//!     fn decode(bytes: BytesMut) -> Result<T, Self::Error>;
//! }
//! ```
//!
//! Let's try implementing our custom [JsonCodec](util::codec::JsonCodec)!
//!
//! ```
//! use serde::{Serialize, de::DeserializeOwned};
//! use bytes::{Bytes, BytesMut};
//! use tsyncp::util::codec::{EncodeMethod, DecodeMethod};
//!
//! #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
//! pub struct CustomJsonCodec;
//!
//! impl<T: Serialize> EncodeMethod<T> for CustomJsonCodec {
//!     type Error = serde_json::Error;
//!
//!     fn encode(data: &T) -> Result<Bytes, Self::Error> {
//!         serde_json::to_vec(data).map(Into::into)
//!     }
//! }
//!
//! impl<T: DeserializeOwned> DecodeMethod<T> for CustomJsonCodec {
//!     type Error = serde_json::Error;
//!
//!     fn decode(bytes: BytesMut) -> Result<T, Self::Error> {
//!         serde_json::from_slice(bytes.as_ref())
//!     }
//! }
//! ```
//!
//! Very simple!
//!
//! Now, if we wanted to use this codec, we can simply specify our channels with them:
//!
//! ```no_run
//! # use color_eyre::{Result, Report};
//! # use serde::{Serialize, Deserialize, de::DeserializeOwned};
//! # use std::time::Duration;
//! # use bytes::{Bytes, BytesMut};
//! # use tsyncp::util::codec::{EncodeMethod, DecodeMethod};
//! use tsyncp::mpsc;
//!
//! # #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
//! # pub struct CustomJsonCodec;
//! # impl<T: Serialize> EncodeMethod<T> for CustomJsonCodec {
//! #     type Error = serde_json::Error;
//! #     fn encode(data: &T) -> Result<Bytes, Self::Error> {
//! #        serde_json::to_vec(data).map(Into::into)
//! #     }
//! # }
//! # impl<T: DeserializeOwned> DecodeMethod<T> for CustomJsonCodec {
//! #     type Error = serde_json::Error;
//! #     fn decode(bytes: BytesMut) -> Result<T, Self::Error> {
//! #         serde_json::from_slice(bytes.as_ref())
//! #     }
//! # }
//! # #[derive(Debug, Serialize, Deserialize)]
//! # struct Dummy {
//! #     field1: String,
//! #     field2: u64,
//! #     field3: Vec<u8>,
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<()> {
//! let rx: mpsc::Receiver<Dummy, CustomJsonCodec> = mpsc::receiver_on("localhost:11114").await?;
//! let tx: mpsc::Sender<Dummy, CustomJsonCodec> = mpsc::sender_to("localhost:11114").await?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! You can view the traits and implementations in [util::codec].

pub mod barrier;
pub mod broadcast;
pub mod channel;
pub mod mpsc;
pub mod multi_channel;
pub mod spsc;
pub mod util;
