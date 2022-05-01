use super::{
    errors::{FrameDecodeSnafu, PollNextSnafu, StreamError},
    Channel,
};
use crate::util::codec::DecodeMethod;
use bytes::BytesMut;
use futures::{ready, Future, StreamExt};
use pin_project::pin_project;
use snafu::ResultExt;
use std::task::Poll;
use tokio::io::AsyncRead;

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
            Some(Err(error)) => return Poll::Ready(Some(Err(error).context(PollNextSnafu))),
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame).context(FrameDecodeSnafu);

        Poll::Ready(Some(decoded))
    }
}

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
            Some(Err(error)) => return Poll::Ready(Some(Err(error).context(PollNextSnafu))),
            None => return Poll::Ready(None),
        };

        Poll::Ready(Some(Ok(frame)))
    }
}
