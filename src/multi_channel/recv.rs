use super::{
    errors::{AcceptingError, AcceptingSnafu, ChannelStreamError, FrameDecodeSnafu, PollNextSnafu},
    Channel,
};
use crate::util::{accept::Accept, codec::DecodeMethod};
use bytes::BytesMut;
use futures::{ready, Future, FutureExt, StreamExt};
use pin_project::pin_project;
use snafu::ResultExt;
use std::{marker::PhantomData, net::SocketAddr, task::Poll};
use tokio::io::AsyncRead;

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
    pub fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn as_frame(self) -> AsFrameFuture<'pin, T, E, N, L> {
        AsFrameFuture {
            channel: self.channel,
        }
    }

    pub fn with_addr(self) -> WithAddrFuture<'pin, T, E, N, L> {
        WithAddrFuture {
            channel: self.channel,
        }
    }

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L> Future for RecvFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<T, ChannelStreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.channel.poll_next_unpin(cx)
    }
}

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
    pub fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn as_frame(self) -> AsFrameWithAddrFuture<'pin, T, E, N, L> {
        AsFrameWithAddrFuture {
            channel: self.channel,
        }
    }

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L> Future for WithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<(T, SocketAddr), ChannelStreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let (frame, addr) = match ready!(self.channel.stream_pool.poll_next_unpin(cx)) {
            Some((Ok(frame), addr)) => (frame, addr),
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(PollNextSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame).with_context(|_| FrameDecodeSnafu { addr });

        Poll::Ready(Some(decoded.map(|d| (d, addr))))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct AsFrameFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AsRef<Channel<T, E, N, L>> for AsFrameFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsMut<Channel<T, E, N, L>> for AsFrameFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsFrameFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    pub fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn with_addr(self) -> AsFrameWithAddrFuture<'pin, T, E, N, L> {
        AsFrameWithAddrFuture {
            channel: self.channel,
        }
    }

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L> Future for AsFrameFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<BytesMut, ChannelStreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let frame = match ready!(self.channel.stream_pool.poll_next_unpin(cx)) {
            Some((Ok(frame), _)) => frame,
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(PollNextSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        Poll::Ready(Some(Ok(frame)))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct AsFrameWithAddrFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AsRef<Channel<T, E, N, L>>
    for AsFrameWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsMut<Channel<T, E, N, L>>
    for AsFrameWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    fn as_mut(&mut self) -> &mut Channel<T, E, N, L> {
        &mut self.channel
    }
}

impl<'pin, T, E, const N: usize, L> AsFrameWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    pub fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L> Future for AsFrameWithAddrFuture<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    type Output = Option<Result<(BytesMut, SocketAddr), ChannelStreamError<E::Error>>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let (frame, addr) = match ready!(self.channel.stream_pool.poll_next_unpin(cx)) {
            Some((Ok(frame), addr)) => (frame, addr),
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(PollNextSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        Poll::Ready(Some(Ok((frame, addr))))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct WhileAcceptingFuture<'pin, T, E, const N: usize, L, Fut>
where
    L: Accept,
    Fut: AsMut<Channel<T, E, N, L>> + Future + Unpin,
{
    #[pin]
    fut: Fut,
    addrs: Vec<SocketAddr>,
    err: Option<AcceptingError<L::Error>>,
    _phantom: PhantomData<(&'pin (), T, E, L)>,
}

impl<'pin, T, E, const N: usize, L, Fut> WhileAcceptingFuture<'pin, T, E, N, L, Fut>
where
    L: Accept,
    Fut: AsRef<Channel<T, E, N, L>> + AsMut<Channel<T, E, N, L>> + Future + Unpin,
{
    pub fn new(fut: Fut) -> Self {
        Self {
            fut,
            addrs: Vec::new(),
            err: None,
            _phantom: PhantomData,
        }
    }

    pub fn should_accept(&self) -> bool {
        let channel_ref = self.fut.as_ref();

        self.err.is_none()
            && channel_ref
                .limit()
                .map(|lim| channel_ref.len() < lim)
                .unwrap_or(true)
    }
}

impl<'pin, T, E, const N: usize, L, Fut> Future for WhileAcceptingFuture<'pin, T, E, N, L, Fut>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead,
    Fut: AsRef<Channel<T, E, N, L>> + AsMut<Channel<T, E, N, L>> + Future + Unpin,
{
    type Output = (
        Fut::Output,
        Result<Vec<SocketAddr>, AcceptingError<L::Error>>,
    );

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let channel_ref = self.fut.as_ref();

        if self.should_accept() {
            if let Poll::Ready(res) = channel_ref
                .listener
                .poll_accept(&channel_ref.stream_config, cx)
            {
                match res.context(AcceptingSnafu) {
                    Ok((s, a)) => {
                        let channel_mut = self.fut.as_mut();
                        channel_mut
                            .stream_pool
                            .push_stream(s, a)
                            .expect("Len is within limit");
                        self.addrs.push(a);
                    }
                    Err(e) => {
                        self.err.replace(e);
                    }
                }
            }
        }

        let output = ready!(self.fut.poll_unpin(cx));
        let addrs = if self.err.is_none() {
            Ok(self.addrs.drain(..).collect())
        } else {
            Err(self.err.take().unwrap())
        };

        Poll::Ready((output, addrs))
    }
}
