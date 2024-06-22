//! Contains futures for receiving items on channel.
//!
//! These futures are built by calling `channel.recv()` and chaining methods to it.
//!
//! For detailed examples, see each documentation in the structs below.

use super::{builder::*, Channel, StreamError, TossStreamErrorItemDecode, TossStreamErrorPollNext};
use crate::util::codec::DecodeMethod;
use bytes::BytesMut;
use futures::{ready, Future, StreamExt};
use pin_project::pin_project;
use std::task::Poll;
use tokio::io::AsyncRead;

/// Future returned by [channel.recv()](crate::channel::Channel::recv)
/// which returns a received item.
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
///     let mut ch: channel::JsonChannel<Dummy> = channel::channel_to("localhost:11114").await?;
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
pub struct RecvFuture<'pin, T, E, S> {
    channel: &'pin mut Channel<T, E, S>,
}

impl<'pin, T, E, S> AsRef<Channel<T, E, S>> for RecvFuture<'pin, T, E, S> {
    fn as_ref(&self) -> &Channel<T, E, S> {
        &self.channel
    }
}

impl<'pin, T, E, S> AsMut<Channel<T, E, S>> for RecvFuture<'pin, T, E, S> {
    fn as_mut(&mut self) -> &mut Channel<T, E, S> {
        &mut self.channel
    }
}

impl<'pin, T, E, S> RecvFuture<'pin, T, E, S> {
    pub(super) fn new(channel: &'pin mut Channel<T, E, S>) -> Self {
        Self { channel }
    }

    /// Returns a new future [AsBytesFuture].
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
    ///     let mut ch: channel::JsonChannel<Dummy> = channel::channel_on("localhost:11114").await?;
    ///
    ///     if let Some(Ok(bytes)) = ch.recv().as_bytes().await {
    ///         println!("{} received", std::str::from_utf8(&bytes).unwrap());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn as_bytes(self) -> AsBytesFuture<'pin, T, E, S> {
        AsBytesFuture::new(self.channel)
    }
}

impl<'pin, T, E, S> Future for RecvFuture<'pin, T, E, S>
where
    E: DecodeMethod<T>,
    S: AsyncRead + Unpin,
{
    type Output = Option<Result<T, StreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let frame = match ready!(self.channel.framed.poll_next_unpin(cx)) {
            Some(Ok(frame)) => frame,
            Some(Err(error)) => {
                return Poll::Ready(Some(Err(error).toss_poll_next_with(|| {
                    (*self.channel.local_addr(), *self.channel.peer_addr())
                })))
            }
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame)
            .toss_item_decode_with(|| (*self.channel.local_addr(), *self.channel.peer_addr()));

        Poll::Ready(Some(decoded))
    }
}

/// Future that returns bytes of an item before decoding.
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
///     let mut ch: channel::JsonChannel<Dummy> = channel::channel_on("localhost:11114").await?;
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
pub struct AsBytesFuture<'pin, T, E, S> {
    channel: &'pin mut Channel<T, E, S>,
}

impl<'pin, T, E, S> AsRef<Channel<T, E, S>> for AsBytesFuture<'pin, T, E, S> {
    fn as_ref(&self) -> &Channel<T, E, S> {
        &self.channel
    }
}

impl<'pin, T, E, S> AsMut<Channel<T, E, S>> for AsBytesFuture<'pin, T, E, S> {
    fn as_mut(&mut self) -> &mut Channel<T, E, S> {
        &mut self.channel
    }
}

impl<'pin, T, E, S> AsBytesFuture<'pin, T, E, S> {
    pub(super) fn new(channel: &'pin mut Channel<T, E, S>) -> Self {
        Self { channel }
    }
}

impl<'pin, T, E, S> Future for AsBytesFuture<'pin, T, E, S>
where
    E: DecodeMethod<T>,
    S: AsyncRead + Unpin,
{
    type Output = Option<Result<BytesMut, StreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let frame = match ready!(self.channel.framed.poll_next_unpin(cx)) {
            Some(Ok(frame)) => frame,
            Some(Err(error)) => {
                return Poll::Ready(Some(Err(error).toss_poll_next_with(|| {
                    (*self.channel.local_addr(), *self.channel.peer_addr())
                })))
            }
            None => return Poll::Ready(None),
        };

        Poll::Ready(Some(Ok(frame)))
    }
}
