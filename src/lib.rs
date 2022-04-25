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
//! Currently, **tsyncp** provides 4 types of channels:
//! * [mpsc]: Multi-producer/single-consumer channel. A consumer can initialize a [Receiver](mpsc::Receiver) by
//! calling [mpsc::recv_on] and passing local address to bind to. Producers can create a [Sender](mpsc::Sender) by
//! calling [mpsc::send_to] and passing the receiver's address to connect to.
//! * [broadcast]: Sigle-producer/multi-consumer channel. A producer can initialize a [Sender](broadcast::Sender) by
//! calling [broadcast::send_on] and passing local address to bind to. Consumers can create a [Receiver](broadcast::Receiver) by
//! calling [broadcast::recv_to] and passing the receiver's address to connect to.
//! * [barrier]: Ensures multiples [Waiter]s connected to a [Barrier] will wait until [Barrier::release](barrier::Barrier::release) is called.
//! A [Barrier](barrier::Barrier) can be initialized by calling [barrier::block_on] and passing
//! local address to bind to. [Waiter]s can be initialized by calling [barrier::wait_to] and
//! passing the barrier's address.
//! * [spsc]: Single-producer/single-consumer channel. A consumer can be initialized either by listening on local address with [spsc::recv_on], or connecting to a sender with [spsc::recv_to]. A producer can be initialized either by listening on a local address with [spsc::send_on], or connecting to a receiver [spsc::send_to].
//!
//! **Note:** If a method ends with `_on` (i.e. `send_on`), it means it's bound and listening on
//! the address parameter; if a method ends with `_to` (i.e. `send_to`), it means that it's connecting to
//! the address parameter.
//!
//! Each module (except for barrier) has a `Sender<T>` type and a `Receiver<T>` type. `Sender` will have
//! async methods such as `send(&mut self, item: T)` for sending data, and `Receiver` will have an async method
//! `recv(&mut self) -> Option<Result<T>>` for receiving data. `barrier` module has a `Waiter` type which
//! can call `wait(&mut self)` to wait for the `Barrier`, and a `Barrier` type that can call `release(&mut self)`
//! to release the waiters.
//!
//! See [examples] to see how they can be used in practice.
//!
//! [Waiter]: crate::barrier::Waiter
//! [Barrier]: crate::barrier::Barrier
//! [examples]: https://github.com/PoOnesNerfect/tsyncp/tree/main/examples
//!
//!
//! # Getting Started
//!
//! ## Dependency and Features
//!
//! Easiest way to use **tsyncp** is to just including it in your `Cargo.toml` file:
//!
//! ```toml
//! tsyncp = { version = "0.1" }
//! ```
//!
//! By default, a few features (`json`, `protobuf`, `bincode`) are enabled for uses in
//! encoding/decoding data. (WIP - only json and protobuf are supported so far)
//!
//! However, if you would like to use other encoding schemes like `rkyv`,
//! you can disable default-features and include it instead:
//!
//! ```toml
//! tsyncp = { version = "0.1", default-features = false, features = ["rkyv"] }
//! ```
//!
//! All possible features are:
//! * [full]: includes all features.
//! * [serde]: includes features that are derivable using serde. (json, bincode)
//! * [json]: serializing/deserializing data as json objects.
//! * [protobuf]: serializing/deserializing data as protobuf objects.
//! * [bincode]: encoding/decoding data as compact bytes. **WIP**
//! * [speedy]: super fast encoding/decoding of data. **WIP**
//! * [rkyv]: super fast encoding/decoding of data and allows zero-copy deserialization. **WIP**
//!
//! [full]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [serde]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [json]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [protobuf]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [bincode]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [speedy]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//! [rkyv]: https://github.com/PoOnesNerfect/tsyncp/blob/main/Cargo.toml#L19
//!
//! ## Initializing Channels
//!
//! We will go through some examples of initializing mpsc's Sender and Receiver and
//! try out a few handy tricks along the way.
//!
//! ### Basic Example
//!
//! This example shows the simpliest way to initialize [mpsc]'s [Sender] and [Receiver].
//!
//! [color-eyre] is used for error handling in the example.
//!
//! [Sender]: crate::mpsc::Sender
//! [Receiver]: crate::mpsc::Receiver
//! [color-eyre]: https://docs.rs/color-eyre/latest/color_eyre/
//!
//! ##### Receiver Side:
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
//!     // This will create a mpsc::Receiver that can accept connections on `localhost:8000`
//!     // and receive json data.
//!     // `JsonReceiver<Dummy>` is a type alias for `Recevier<Dummy, JsonCodec>`.
//!     // `recv_on(..)` can take any parameter type that implements `ToSocketAddrs`.
//!     let mut receiver: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:8000").await?;
//!
//!     // You have to accept a connection to receive items!
//!     // Calling `recv()` without accepting any connection will immediately return `None`.
//!     receiver.accept().await?;
//!
//!     if let Some(item) = receiver.recv().await {
//!         let item = item?;
//!
//!         println!("received item: {item:?}");
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ##### Sender Side:
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
//!     // Connect to the receiver listening on `localhost:8000` with `send_to` function.
//!     // `JsonSender<Dummy>` is a type alias for `Sender<Dummy, JsonCodec>`.
//!     // If the receiver is not initialized and accepting connections, it will return
//!     // `ConnectionRefused`.
//!     let mut sender: mpsc::JsonSender<Dummy> = mpsc::send_to("localhost:8000").await?;
//!
//!     let dummy = Dummy {
//!         field1: String::from("hello world"),
//!         field2: 1234567,
//!         field3: vec![1, 2, 3, 4]
//!     };
//!
//!     sender.send(dummy).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! That wasn't so hard!
//!
//! However, there are couple things that could make it annoying to use:
//! * For the receiver, after initializing, you have to call `receiver.accept().await?` in a separate line to accept a connection,
//! which can be annoying, especially if you want to accept multiple connections before receiving data.
//! * For the sender, before running application, you have to make sure receiver application is running and accepting connections,
//! or else sender application will return `ConnectionRefused` error.
//!
//! We can solve these issues easily, as shown in the below example.
//!
//! ### Example: Appending `.accept()` and `.retry()` to builder futures.
//!
//! ##### Receiver Side:
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
//!     // This will create a `mpsc::Receiver` and accept `1` connection on `localhost:8000` before returning the
//!     // receiver. This way, there's no need to write a separate line(s) to accept connections, though you
//!     // still can.
//!     let mut receiver: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:8000").accept(1).await?;
//!
//!     // no need for separate accepting anymore.
//!     // receiver.accept().await?;
//!
//!     if let Some(item) = receiver.recv().await {
//!         let item = item?;
//!
//!         println!("received item: {item:?}");
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ##### Sender Side:
//!
//! ```no_run
//! use color_eyre::Result;
//! use serde::{Serialize, Deserialize};
//! use std::time::Duration;
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
//!     let retry_interval = Duration::from_millis(500);
//!     let max_retries = 100;
//!
//!     // This will create a `mpsc::Sender` and retry initializing a connection to `localhost:8000`
//!     // 100 times with interval of 500 ms. No more `ConnectionRefused`!
//!     let mut sender: mpsc::JsonSender<Dummy> = mpsc::send_to("localhost:8000")
//!         .retry(retry_interval, max_retries)
//!         .await?;
//!
//!     let dummy = Dummy {
//!         field1: String::from("hello world"),
//!         field2: 1234567,
//!         field3: vec![1, 2, 3, 4]
//!     };
//!
//!     sender.send(dummy).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! That was also not hard at all!
//!
//! But how does it work?
//!
//! Functions like [mpsc::recv_on] and [mpsc::send_to] actually return
//! [ReceiverBuilderFuture](mpsc::builder::ReceiverBuilderFuture)
//! and [SenderBuilderFuture](mpsc::builder::SenderBuilderFuture).
//! Quite a mouthful I know, but, what it means is that, the returned future is also a
//! builder to set configurations before `await`ing it.
//!
//! For `SenderBuilderFuture`, you can append a configuration like [retry](mpsc::builder::SenderBuilderFuture::retry) as well as any TCP
//! configs like [reuseaddr](mpsc::builder::SenderBuilderFuture::set_tcp_reuseaddr),
//! [linger](mpsc::builder::SenderBuilderFuture::set_tcp_linger) and [nodelay](mpsc::builder::SenderBuilderFuture::set_tcp_nodelay), etc.
//!
//! For `ReceiverBuilderFuture`, you can append configurations like
//! [accept(n: usize)](mpsc::builder::ReceiverBuilderFuture::accept),
//! [accept_filtered(Fn(SocketAddr) -> bool)](mpsc::builder::ReceiverBuilderFuture::accept_filtered),
//! [limit(n: usize)](mpsc::builder::ReceiverBuilderFuture::limit), etc.,
//! as well as all the TCP configs mentioned above.
//!
//! See [ReceiverBuilderFuture](mpsc::builder::ReceiverBuilderFuture) and
//! [SenderBuilderFuture](mpsc::builder::SenderBuilderFuture) for all configuration options.
//!
//! WIP. More documentations and examples to come!

pub mod barrier;
pub mod broadcast;
pub mod channel;
pub mod mpsc;
pub mod multi_channel;
pub mod spsc;
pub mod util;
