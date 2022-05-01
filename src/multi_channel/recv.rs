use super::{
    accept::WhileAcceptingFuture,
    errors::{ChannelStreamError, FrameDecodeSnafu, PollNextSnafu},
    Channel,
};
use crate::util::{accept::Accept, codec::DecodeMethod};
use bytes::BytesMut;
use futures::{ready, Future, StreamExt};
use pin_project::pin_project;
use snafu::ResultExt;
use std::{net::SocketAddr, task::Poll};
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
    pub(super) fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn as_bytes(self) -> AsBytesFuture<'pin, T, E, N, L> {
        AsBytesFuture::new(self.channel)
    }

    pub fn with_addr(self) -> WithAddrFuture<'pin, T, E, N, L> {
        WithAddrFuture::new(self.channel)
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
    fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn as_bytes(self) -> AsBytesWithAddrFuture<'pin, T, E, N, L> {
        AsBytesWithAddrFuture::new(self.channel)
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

    pub fn with_addr(self) -> AsBytesWithAddrFuture<'pin, T, E, N, L> {
        AsBytesWithAddrFuture::new(self.channel)
    }

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L> Future for AsBytesFuture<'pin, T, E, N, L>
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

    pub fn accepting(self) -> WhileAcceptingFuture<'pin, T, E, N, L, Self>
    where
        E: DecodeMethod<T>,
        L::Output: AsyncRead + Unpin,
    {
        WhileAcceptingFuture::new(self)
    }
}

impl<'pin, T, E, const N: usize, L> Future for AsBytesWithAddrFuture<'pin, T, E, N, L>
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
