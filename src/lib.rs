#![warn(
    missing_debug_implementations,
    missing_docs,
    rust_2018_idioms,
    unreachable_pub
)]
#![warn(rustdoc::missing_doc_code_examples)]

//! Synchronization primitives over TCP for message passing between services.
//!
//! Tsyncp allows customizing different Serialization/Deserialization methods to encode/decode
//! data; currently, supported schemes straight from the library are *Json*, *Protobuf*, *Bincode*, *Speedy*, and *Rkyv*; however,
//! users can very easily implement their own [EncodeMethod] and [DecodeMethod].
//!
//! *Warning: Tsyncp is not a message-broker nor it tries to be;
//! it's just a simple message-passer for simple and convenient use cases.*
//!
//! [std]: https://doc.rust-lang.org/stable/std/
//! [tokio]: https://docs.rs/tokio/latest/tokio/index.html
//! [EncodeMethod]: crate::util::codec::EncodeMethod
//! [DecodeMethod]: crate::util::codec::DecodeMethod
//!
//! ## Provided APIs
//! Currently, **tsyncp** provides 4 types of channels:
//! * [mpsc]: Multi-producer/single-consumer channel. A consumer listens to incoming connections and receives items from
//! connected TCP streams. Any/configured number of producers can connect to the listener's socket address
//! and send items.
//! * [broadcast]: Sigle-producer/multi-consumer channel. A producer listens to incoming
//! connections and broadcasts items to all connected TCP streams. Any/configured number of consumers can
//! connect to the producer and receive broadcasted items.
//! * [barrier]: Ensures multiples [Waiters] connected to a [Barrier] will wait until the it
//! releases the lock.
//! * [spsc]: Single-producer/single-consumer channel. A consumer can connect to or listen to a
//! single connection and receive items from it. A producer can connect to or listen to a single
//! connection and send items to it.
//!
//! See [examples] to see how they can be used in practice.
//!
//! [Waiters]: crate::barrier::Waiter
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
//! encoding/decoding data.
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
//! * [bincode]: encoding/decoding data as compact bytes.
//! * [speedy]: super fast encoding/decoding of data.
//! * [rkyv]: super fast encoding/decoding of data and allows zero-copy deserialization.
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
//! We will go through initializing mpsc's Sender and Receiver and try out a few handy tricks.
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
//!     // `recv_on(..)` can take any parameter types that implements `ToSocketAddrs`.
//!     let mut receiver: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:8000").await?;
//!
//!     // Accept a connection to receive items!
//!     // Not accepting any connection before calling `recv()` will returning `None`.
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
//!     // After receiver application is running,
//!     // connect to receiver listening on `localhost:8000` with `send_to` function.
//!     // `JsonSender<Dummy>` is a type alias for `Sender<Dummy, JsonCodec>`.
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
//! That was so easy!
//!
//! However, there are couple things that makes it annoying to use:
//! * You have to call `receiver.accept().await?` in another line to accept connection
//! after initializing the receiver, which can be annoying.
//! * Before running the sender application, you have to make sure receiver application is running and accepting connections,
//! or else sender application will return `ConnectionRefused` error.
//!
//! We can solve these issues easily, shown in the below example.
//!
//! ### Example appending `.accept()` and `.retry()` to futures.
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
//!     // This will create a mpsc::Receiver and wait to accept 1 connection on `localhost:8000` before returning the
//!     // receiver. This way, there's no need to write a new line to accept a connection, though you
//!     // still can.
//!     let mut receiver: mpsc::JsonReceiver<Dummy> = mpsc::recv_on("localhost:8000").accept(1).await?;
//!
//!     // no need for separate accepting
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
//!     // This will create a Sender and retry initializing a connection to `localhost:8000`
//!     // 100 times with interval of 500 ms.
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
//! If your code is CPU-bound and you wish to limit the number of threads used
//! to run it, you should use a separate thread pool dedicated to CPU bound tasks.
//! For example, you could consider using the [rayon] library for CPU-bound
//! tasks. It is also possible to create an extra Tokio runtime dedicated to
//! CPU-bound tasks, but if you do this, you should be careful that the extra
//! runtime runs _only_ CPU-bound tasks, as IO-bound tasks on that runtime
//! will behave poorly.
//!
//! Hint: If using rayon, you can use a [`oneshot`] channel to send the result back
//! to Tokio when the rayon task finishes.
//!
//! [rayon]: https://docs.rs/rayon
//! [`oneshot`]: crate::sync::oneshot
//!
//! ## Asynchronous IO
//!
//! As well as scheduling and running tasks, Tokio provides everything you need
//! to perform input and output asynchronously.
//!
//! The [`tokio::io`] module provides Tokio's asynchronous core I/O primitives,
//! the [`AsyncRead`], [`AsyncWrite`], and [`AsyncBufRead`] traits. In addition,
//! when the "io-util" feature flag is enabled, it also provides combinators and
//! functions for working with these traits, forming as an asynchronous
//! counterpart to [`std::io`].
//!
//! Tokio also includes APIs for performing various kinds of I/O and interacting
//! with the operating system asynchronously. These include:
//!
//! * [`tokio::net`], which contains non-blocking versions of [TCP], [UDP], and
//!   [Unix Domain Sockets][UDS] (enabled by the "net" feature flag),
//! * [`tokio::fs`], similar to [`std::fs`] but for performing filesystem I/O
//!   asynchronously (enabled by the "fs" feature flag),
//! * [`tokio::signal`], for asynchronously handling Unix and Windows OS signals
//!   (enabled by the "signal" feature flag),
//! * [`tokio::process`], for spawning and managing child processes (enabled by
//!   the "process" feature flag).
//!
//! [`tokio::io`]: crate::io
//! [`AsyncRead`]: crate::io::AsyncRead
//! [`AsyncWrite`]: crate::io::AsyncWrite
//! [`AsyncBufRead`]: crate::io::AsyncBufRead
//! [`std::io`]: std::io
//! [`tokio::net`]: crate::net
//! [TCP]: crate::net::tcp
//! [UDP]: crate::net::UdpSocket
//! [UDS]: crate::net::unix
//! [`tokio::fs`]: crate::fs
//! [`std::fs`]: std::fs
//! [`tokio::signal`]: crate::signal
//! [`tokio::process`]: crate::process
//!
//! # Examples
//!
//! A simple TCP echo server:
//!
//! ```no_run
//! use tokio::net::TcpListener;
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let listener = TcpListener::bind("127.0.0.1:8080").await?;
//!
//!     loop {
//!         let (mut socket, _) = listener.accept().await?;
//!
//!         tokio::spawn(async move {
//!             let mut buf = [0; 1024];
//!
//!             // In a loop, read data from the socket and write the data back.
//!             loop {
//!                 let n = match socket.read(&mut buf).await {
//!                     // socket closed
//!                     Ok(n) if n == 0 => return,
//!                     Ok(n) => n,
//!                     Err(e) => {
//!                         eprintln!("failed to read from socket; err = {:?}", e);
//!                         return;
//!                     }
//!                 };
//!
//!                 // Write the data back
//!                 if let Err(e) = socket.write_all(&buf[0..n]).await {
//!                     eprintln!("failed to write to socket; err = {:?}", e);
//!                     return;
//!                 }
//!             }
//!         });
//!     }
//! }
//! ```
//!
//! ## Feature flags
//!
//! Tokio uses a set of [feature flags] to reduce the amount of compiled code. It
//! is possible to just enable certain features over others. By default, Tokio
//! does not enable any features but allows one to enable a subset for their use
//! case. Below is a list of the available feature flags. You may also notice
//! above each function, struct and trait there is listed one or more feature flags
//! that are required for that item to be used. If you are new to Tokio it is
//! recommended that you use the `full` feature flag which will enable all public APIs.
//! Beware though that this will pull in many extra dependencies that you may not
//! need.
//!
//! - `full`: Enables all features listed below except `test-util` and `tracing`.
//! - `rt`: Enables `tokio::spawn`, the basic (current thread) scheduler,
//!         and non-scheduler utilities.
//! - `rt-multi-thread`: Enables the heavier, multi-threaded, work-stealing scheduler.
//! - `io-util`: Enables the IO based `Ext` traits.
//! - `io-std`: Enable `Stdout`, `Stdin` and `Stderr` types.
//! - `net`: Enables `tokio::net` types such as `TcpStream`, `UnixStream` and
//!          `UdpSocket`, as well as (on Unix-like systems) `AsyncFd` and (on
//!          FreeBSD) `PollAio`.
//! - `time`: Enables `tokio::time` types and allows the schedulers to enable
//!           the built in timer.
//! - `process`: Enables `tokio::process` types.
//! - `macros`: Enables `#[tokio::main]` and `#[tokio::test]` macros.
//! - `sync`: Enables all `tokio::sync` types.
//! - `signal`: Enables all `tokio::signal` types.
//! - `fs`: Enables `tokio::fs` types.
//! - `test-util`: Enables testing based infrastructure for the Tokio runtime.
//!
//! _Note: `AsyncRead` and `AsyncWrite` traits do not require any features and are
//! always available._
//!
//! ### Internal features
//!
//! These features do not expose any new API, but influence internal
//! implementation aspects of Tokio, and can pull in additional
//! dependencies.
//!
//! - `parking_lot`: As a potential optimization, use the _parking_lot_ crate's
//! synchronization primitives internally. MSRV may increase according to the
//! _parking_lot_ release in use.
//!
//! ### Unstable features
//!
//! These feature flags enable **unstable** features. The public API may break in 1.x
//! releases. To enable these features, the `--cfg tokio_unstable` must be passed to
//! `rustc` when compiling. This is easiest done using the `RUSTFLAGS` env variable:
//! `RUSTFLAGS="--cfg tokio_unstable"`.
//!
//! - `tracing`: Enables tracing events.
//!
//! [feature flags]: https://doc.rust-lang.org/cargo/reference/manifest.html#the-features-section

pub mod barrier;
pub mod broadcast;
pub mod channel;
pub mod mpsc;
pub mod multi_channel;
pub mod spsc;
pub mod util;
