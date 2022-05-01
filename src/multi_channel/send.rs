use super::{
    accept::WhileAcceptingFuture,
    errors::{ChannelSinkError, ItemEncodeSnafu, SendFilteredSnafu},
    Channel,
};
use crate::util::{accept::Accept, codec::EncodeMethod};
use futures::SinkExt;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::ResultExt;
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

    pub fn filtered<F>(self, filter: F) -> SendFilteredFuture<'pin, T, E, N, L, F>
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

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        T: Clone,
        E: EncodeMethod<T>,
        L::Output: AsyncWrite + Unpin,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L> Future for SendFuture<'pin, T, E, N, L>
where
    T: Clone,
    L: Accept,
    E: EncodeMethod<T>,
    L::Output: AsyncWrite + Unpin,
{
    type Output = Result<(), ChannelSinkError<E::Error>>;

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

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        T: Clone,
        E: EncodeMethod<T>,
        L::Output: AsyncWrite + Unpin,
        F: Fn(SocketAddr) -> bool,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L, F> Future for SendFilteredFuture<'pin, T, E, N, L, F>
where
    T: Clone,
    L: Accept,
    E: EncodeMethod<T>,
    L::Output: AsyncWrite + Unpin,
    F: Fn(SocketAddr) -> bool,
{
    type Output = Result<(), ChannelSinkError<E::Error>>;

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

                    let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

                    this.channel
                        .stream_pool
                        .start_send_filtered(encoded, &this.filter);

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
                                .context(SendFilteredSnafu),
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
                .context(SendFilteredSnafu),
        )
    }
}
