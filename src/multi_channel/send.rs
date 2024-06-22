//! Contains futures for sending items on channel.
//!
//! These futures are built by calling `channel.send(item)` and chaining methods to it.
//!
//! For detailed examples, see each documentation in the structs below.

use super::{accept::ChainedAcceptFuture, Channel};
use super::{SinkError, TossSinkErrorItemEncode, TossSinkErrorSinkErrors};
use crate::util::{codec::EncodeMethod, Accept};
use futures::SinkExt;
use futures::{ready, Future};
use pin_project::pin_project;
use std::{net::SocketAddr, task::Poll};
use tokio::io::AsyncWrite;

#[derive(Debug, Clone, Copy)]
enum State {
    Ready,
    StartSend,
    Flush,
    Done,
}

impl State {
    fn new() -> Self {
        State::Ready
    }

    fn next(self) -> Self {
        match self {
            State::Ready => State::StartSend,
            State::StartSend => State::Flush,
            State::Flush => State::Done,
            State::Done => Self::Done,
        }
    }
}

/// Basic future that sends an item to all connections.
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
///         field2: 123123,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     ch.send(dummy).await?;
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project]
pub struct SendFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    state: State,
    item: Option<T>,
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AsRef<Channel<T, E, N, L>> for SendFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsMut<Channel<T, E, N, L>> for SendFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L> SendFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    pub(super) fn new(channel: &'pin mut Channel<T, E, N, L>, item: T) -> Self {
        Self {
            channel,
            item: Some(item),
            state: State::Ready,
        }
    }

    /// Returns a new future [SendFilteredFuture] with predefined closure `|a| addrs.contains(&a)`.
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::multi_channel;
    /// use std::net::SocketAddrV4;
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
    ///         field2: 123123,
    ///         field3: vec![1, 2, 3, 4]
    ///     };
    ///
    ///     let addrs = ["127.0.0.1:8000".parse::<SocketAddrV4>()?.into()];
    ///
    ///     ch.send(dummy).to(&addrs).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn to(
        self,
        addrs: &[SocketAddr],
    ) -> SendFilteredFuture<'pin, T, E, N, L, impl Fn(SocketAddr) -> bool + '_> {
        let Self {
            mut item, channel, ..
        } = self;

        SendFilteredFuture::new(
            channel,
            item.take().expect("Item is only taken during await"),
            |a| addrs.contains(&a),
        )
    }

    /// Returns a new future [SendFilteredFuture].
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
    ///         field2: 123123,
    ///         field3: vec![1, 2, 3, 4]
    ///     };
    ///
    ///     ch.send(dummy).filter(|a| a.port() % 2 == 0).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn filter<F>(self, filter: F) -> SendFilteredFuture<'pin, T, E, N, L, F>
    where
        F: Fn(SocketAddr) -> bool,
    {
        let Self {
            mut item, channel, ..
        } = self;

        SendFilteredFuture::new(
            channel,
            item.take().expect("Item is only taken during await"),
            filter,
        )
    }

    /// Returns [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) that
    /// wraps this future.
    ///
    /// [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture) will poll accept
    /// whenever this future is polled. When this future completes, [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture)
    /// will also complete, whether or not it accepted any connections.
    ///
    /// To see available chain methods, see [ChainedAcceptFuture](crate::multi_channel::accept::ChainedAcceptFuture).
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
    ///         field2: 123123,
    ///         field3: vec![1, 2, 3, 4]
    ///     };
    ///
    ///     let (res, accepted_res) = ch.send(dummy).accepting().await;
    ///
    ///     let num = accepted_res?;
    ///     println!("accepted {num} connections");
    ///
    ///     res?;
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
        E: EncodeMethod<T>,
        L::Output: AsyncWrite + Unpin,
    {
        ChainedAcceptFuture::new(self, |_| {}, |_| true)
    }
}

impl<'pin, T, E, const N: usize, L> Future for SendFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: EncodeMethod<T>,
    L::Output: AsyncWrite + Unpin,
{
    type Output = Result<(), SinkError<E::Error>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        loop {
            match self.state {
                State::Ready => {
                    ready!(self.channel.poll_ready_unpin(cx))?;
                    self.state = State::StartSend;
                }
                State::StartSend => {
                    let item = self
                        .item
                        .take()
                        .expect("taking item should only be called once");

                    self.channel.start_send_unpin(item)?;
                    self.state = State::Flush;
                }
                State::Flush => {
                    ready!(self.channel.poll_flush_unpin(cx))?;
                    self.state = State::Done;
                }
                State::Done => break,
            }
        }

        Poll::Ready(Ok(()))
    }
}

#[derive(Debug, Clone, Copy)]
enum FilterState {
    GetIndices,
    Poll,
}

impl FilterState {
    fn new() -> Self {
        FilterState::GetIndices
    }
}

/// Basic future that sends an item to addresses filter by the given closure.
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
///         field2: 123123,
///         field3: vec![1, 2, 3, 4]
///     };
///
///     ch.send(dummy).filter(|a| a.port() % 2 == 0).await?;
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project]
pub struct SendFilteredFuture<'pin, T, E, const N: usize, L, F>
where
    L: Accept,
{
    state: State,
    filter_state: FilterState,
    item: Option<T>,
    filter: F,
    indices: Vec<usize>,
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L, F> AsRef<Channel<T, E, N, L>>
    for SendFilteredFuture<'pin, T, E, N, L, F>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L, F> AsMut<Channel<T, E, N, L>>
    for SendFilteredFuture<'pin, T, E, N, L, F>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L, F> SendFilteredFuture<'pin, T, E, N, L, F>
where
    L: Accept,
{
    fn new(channel: &'pin mut Channel<T, E, N, L>, item: T, filter: F) -> Self {
        Self {
            indices: Vec::with_capacity(channel.len()),
            channel,
            item: Some(item),
            filter_state: FilterState::new(),
            state: State::new(),
            filter,
        }
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
    ///         .accept()
    ///         .num(10)
    ///         .await?;
    ///
    ///     let dummy = Dummy {
    ///         field1: String::from("hello world"),
    ///         field2: 123123,
    ///         field3: vec![1, 2, 3, 4]
    ///     };
    ///
    ///     let (res, accepted_res) = ch
    ///         .send(dummy)
    ///         .filter(|a| a.port() % 2 == 0)
    ///         .accepting().await;
    ///
    ///     let num = accepted_res?;
    ///     println!("accepted {num} connections");
    ///
    ///     res?;
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
        E: EncodeMethod<T>,
        L::Output: AsyncWrite + Unpin,
        F: Fn(SocketAddr) -> bool,
    {
        ChainedAcceptFuture::new(self, |_| {}, |_| true)
    }
}

impl<'pin, T, E, const N: usize, L, F> Future for SendFilteredFuture<'pin, T, E, N, L, F>
where
    L: Accept,
    E: EncodeMethod<T>,
    L::Output: AsyncWrite + Unpin,
    F: Fn(SocketAddr) -> bool,
{
    type Output = Result<(), SinkError<E::Error>>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();

        // fist state is (State::Ready, FilterState::GetIndices)
        loop {
            match (this.state, this.filter_state) {
                (State::Ready, FilterState::Poll) => {
                    let channel = &mut this.channel;
                    let mut indices = &mut this.indices;

                    ready!(channel.stream_pool.poll_ready_indices(&mut indices, cx));

                    // go to start_send state without renewing indices
                    this.state = this.state.next();
                }
                (State::StartSend, _) => {
                    let item = this
                        .item
                        .take()
                        .expect("taking item should only be called once");

                    let encoded = E::encode(&item).toss_item_encode(*this.channel.local_addr())?;

                    this.channel
                        .stream_pool
                        .start_send_filter(encoded, &this.filter);

                    // go to next state and get indices first
                    this.state = this.state.next();
                    this.filter_state = FilterState::GetIndices;
                }
                (State::Flush, FilterState::Poll) => {
                    let channel = &mut this.channel;
                    let mut indices = &mut this.indices;

                    ready!(channel.stream_pool.poll_flush_indices(&mut indices, cx));
                    this.state = State::Done;
                }
                (State::Done, _) => break,
                (_, FilterState::GetIndices) => {
                    let len = this.channel.len();

                    // if len is 0, no reason to continue,
                    // return and dump all errors
                    if len == 0 {
                        return Poll::Ready(
                            this.channel
                                .stream_pool
                                .drain_sink_res()
                                .toss_sink_errors(*this.channel.local_addr()),
                        );
                    }

                    this.indices = this.channel.stream_pool.get_indices(&this.filter);

                    this.filter_state = FilterState::Poll;
                }
            }
        }

        Poll::Ready(
            this.channel
                .stream_pool
                .drain_sink_res()
                .toss_sink_errors(*this.channel.local_addr()),
        )
    }
}
