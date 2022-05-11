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
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, S> Sender<T, E, S>
where
    E: EncodeMethod<T>,
    S: AsyncWrite + Unpin,
{
    pub async fn send(&mut self, item: T) -> Result<(), channel::errors::SinkError<E::Error>> {
        self.0.send(item).await
    }
}

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
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn limit(&self) -> Option<usize> {
        self.0.limit()
    }

    pub fn local_addr(&self) -> &SocketAddr {
        self.0.local_addr()
    }

    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.0.peer_addrs()
    }
}

impl<T, E, const N: usize, L> Receiver<T, E, N, L>
where
    L: Accept,
    L::Error: 'static,
{
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
    pub fn recv(&mut self) -> RecvFuture<'_, T, E, N, L> {
        self.0.recv()
    }
}
