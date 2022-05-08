//! Contains futures for receiving items on channel.
//!
//! These futures are built by calling `channel.recv()` and chaining methods to it.
//!
//! For detailed examples, see each documentation in the structs below.

use super::{
    accept::ChainedAcceptFuture,
    errors::{ItemDecodeSnafu, StreamError, StreamSnafu},
    Channel,
};
use crate::util::{codec::DecodeMethod, Accept};
use bytes::BytesMut;
use futures::{ready, Future, StreamExt};
use pin_project::pin_project;
use snafu::ResultExt;
use std::{net::SocketAddr, task::Poll};
use tokio::io::AsyncRead;

/// Basic future that returns a received item.
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
///         .accept(10)
///         .await?;
///
///     if let Some(Ok(item)) = ch.recv().await {
///         println!("{item:?} received");
///     }
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project]
pub struct RecvFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AsRef<Channel<T, E, N, L>> for RecvFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsMut<Channel<T, E, N, L>> for RecvFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L> RecvFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    pub(super) fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    /// Returns a new future [AsBytesFuture].
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
    ///         .accept(10)
    ///         .await?;
    ///
    ///     if let Some(Ok(bytes)) = ch.recv().as_bytes().await {
    ///         println!("{} received", std::str::from_utf8(&bytes).unwrap());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn as_bytes(self) -> AsBytesFuture<'pin, T, E, N, L> {
        AsBytesFuture::new(self.channel)
    }

    /// Returns a new future [WithAddrFuture].
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
    ///         .accept(10)
    ///         .await?;
    ///
    ///     if let Some(Ok((item, addr))) = ch.recv().with_addr().await {
    ///         println!("{item:?} received from {addr}");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn with_addr(self) -> WithAddrFuture<'pin, T, E, N, L> {
        WithAddrFuture::new(self.channel)
    }

    /// Returns [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) that
    /// wraps this future.
    ///
    /// [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) will poll accept
    /// whenever this future is polled. When this future completes, [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture)
    /// will also complete, whether or not it accepted any connections.
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
    ///         .accept(10)
    ///         .await?;
    ///
    ///     if let (Some(Ok(item)), Ok(accepted_addrs)) = ch.recv().accepting().await {
    ///         for addr in accepted_addrs {
    ///             println!("{addr} accepted while receiving item")
    ///         }
    ///
    ///         println!("{item:?} received");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn accepting(
        self,
    ) -> ChainedAcceptFuture<
        'pin,
        T,
        E,
        N,
        L,
        Self,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    >
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        ChainedAcceptFuture::new(self, |_| {}, |_| true)
    }
}

impl<'pin, T, E, const N: usize, L> Future for RecvFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<T, StreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.channel.poll_next_unpin(cx)
    }
}

/// Future that returns a tuple of received item and the address it came from.
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
///         .accept(10)
///         .await?;
///
///     if let Some(Ok((item, addr))) = ch.recv().with_addr().await {
///         println!("{item:?} received from {addr}");
///     }
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project]
pub struct WithAddrFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AsRef<Channel<T, E, N, L>> for WithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsMut<Channel<T, E, N, L>> for WithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L> WithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    /// Returns a new future [AsBytesWithAddrFuture].
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
    ///         .accept(10)
    ///         .await?;
    ///
    ///     if let Some(Ok((bytes, addr))) = ch.recv().with_addr().as_bytes().await {
    ///         println!("{} received from {addr}", std::str::from_utf8(&bytes).unwrap());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn as_bytes(self) -> AsBytesWithAddrFuture<'pin, T, E, N, L> {
        AsBytesWithAddrFuture::new(self.channel)
    }

    /// Returns [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) that
    /// wraps this future.
    ///
    /// [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) will poll accept
    /// whenever this future is polled. When this future completes, [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture)
    /// will also complete, whether or not it accepted any connections.
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
    ///         .accept(10)
    ///         .await?;
    ///
    ///     if let (Some(Ok((item, addr))), Ok(accepted_addrs)) = ch.recv().with_addr().accepting().await {
    ///         for addr in accepted_addrs {
    ///             println!("{addr} accepted while receiving item")
    ///         }
    ///
    ///         println!("{item:?} received from {addr}");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn accepting(
        self,
    ) -> ChainedAcceptFuture<
        'pin,
        T,
        E,
        N,
        L,
        Self,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    >
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        ChainedAcceptFuture::new(self, |_| {}, |_| true)
    }
}

impl<'pin, T, E, const N: usize, L> Future for WithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<(T, SocketAddr), StreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let (frame, addr) = match ready!(self.channel.stream_pool.poll_next_unpin(cx)) {
            Some((Ok(frame), addr)) => (frame, addr),
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(StreamSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame).with_context(|_| ItemDecodeSnafu { addr });

        Poll::Ready(Some(decoded.map(|d| (d, addr))))
    }
}

/// Future that returns bytes of an item before decoding.
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
///         .accept(10)
///         .await?;
///
///     if let Some(Ok(bytes)) = ch.recv().as_bytes().await {
///         println!("{} received", std::str::from_utf8(&bytes).unwrap());
///     }
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project]
pub struct AsBytesFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AsRef<Channel<T, E, N, L>> for AsBytesFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsMut<Channel<T, E, N, L>> for AsBytesFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsBytesFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    /// Returns a new future [AsBytesWithAddrFuture].
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
    ///         .accept(10)
    ///         .await?;
    ///
    ///     if let Some(Ok((bytes, addr))) = ch.recv().as_bytes().with_addr().await {
    ///         println!("{} received from {addr}", std::str::from_utf8(&bytes).unwrap());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn with_addr(self) -> AsBytesWithAddrFuture<'pin, T, E, N, L> {
        AsBytesWithAddrFuture::new(self.channel)
    }

    /// Returns [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) that
    /// wraps this future.
    ///
    /// [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) will poll accept
    /// whenever this future is polled. When this future completes, [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture)
    /// will also complete, whether or not it accepted any connections.
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
    ///         .accept(10)
    ///         .await?;
    ///
    ///     if let (Some(Ok(bytes)), Ok(accepted_addrs)) = ch.recv().as_bytes().accepting().await {
    ///         for addr in accepted_addrs {
    ///             println!("{addr} accepted while receiving item")
    ///         }
    ///
    ///         println!("{} received", std::str::from_utf8(&bytes).unwrap());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn accepting(
        self,
    ) -> ChainedAcceptFuture<
        'pin,
        T,
        E,
        N,
        L,
        Self,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    >
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        ChainedAcceptFuture::new(self, |_| {}, |_| true)
    }
}

impl<'pin, T, E, const N: usize, L> Future for AsBytesFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<BytesMut, StreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let frame = match ready!(self.channel.stream_pool.poll_next_unpin(cx)) {
            Some((Ok(frame), _)) => frame,
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(StreamSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        Poll::Ready(Some(Ok(frame)))
    }
}

/// Future that returns a tuple of bytes and address where it came from.
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
///         .accept(10)
///         .await?;
///
///     if let Some(Ok((bytes, addr))) = ch.recv().as_bytes().with_addr().await {
///         println!("{} received from {addr}", std::str::from_utf8(&bytes).unwrap());
///     }
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project]
pub struct AsBytesWithAddrFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AsRef<Channel<T, E, N, L>>
    for AsBytesWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsMut<Channel<T, E, N, L>>
    for AsBytesWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsBytesWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn accepting(
        self,
    ) -> ChainedAcceptFuture<
        'pin,
        T,
        E,
        N,
        L,
        Self,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    >
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        ChainedAcceptFuture::new(self, |_| {}, |_| true)
    }
}

impl<'pin, T, E, const N: usize, L> Future for AsBytesWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<(BytesMut, SocketAddr), StreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let (frame, addr) = match ready!(self.channel.stream_pool.poll_next_unpin(cx)) {
            Some((Ok(frame), addr)) => (frame, addr),
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(StreamSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        Poll::Ready(Some(Ok((frame, addr))))
    }
}
