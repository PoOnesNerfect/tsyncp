use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::split::{RWSplit, TcpSplit};
use crate::{broadcast, channel, mpsc};
use errors::*;
use futures::Sink;
use futures::Stream;
use futures::{ready, StreamExt};
use futures::{Future, SinkExt};
use snafu::{Backtrace, ResultExt};
use std::fmt;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};

pub mod builder;

#[cfg(feature = "json")]
pub type JsonSender<T> = Sender<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufSender<T> = Sender<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvSender<T> = Sender<T, crate::util::codec::RkyvCodec>;

#[cfg(feature = "json")]
pub type JsonReceiver<T> = Receiver<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufReceiver<T> = Receiver<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::RkyvCodec>;

pub fn send_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::SenderToBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::sender_to(dest)
}

pub fn send_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::SenderOnBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::sender_on(local_addr)
}

pub fn recv_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::ReceiverToBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::receiver_to(dest)
}

pub fn recv_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ReceiverOnBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::receiver_on(local_addr)
}

#[pin_project::pin_project]
pub struct Sender<T: fmt::Debug, E, RW = TcpSplit>(#[pin] pub(crate) channel::Channel<T, E, RW>);

impl<T: fmt::Debug, E, RW> From<mpsc::Sender<T, E, RW>> for Sender<T, E, RW> {
    fn from(sender: mpsc::Sender<T, E, RW>) -> Self {
        Sender(sender.0)
    }
}

impl<T: fmt::Debug, E, RW> From<channel::Channel<T, E, RW>> for Sender<T, E, RW> {
    fn from(channel: channel::Channel<T, E, RW>) -> Self {
        Sender(channel)
    }
}

impl<T, E, RW> Sender<T, E, RW>
where
    T: fmt::Debug + Clone,
{
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, R, W> Sender<T, E, RWSplit<R, W>>
where
    T: fmt::Debug,
{
    pub fn split(
        self,
    ) -> Result<(Self, Receiver<T, E, RWSplit<R, W>>), channel::errors::SplitError> {
        let (r, w) = self.0.split()?;

        Ok((w, r))
    }
}

impl<T, E, RW> Sender<T, E, RW>
where
    T: 'static + fmt::Debug,
    E: 'static + EncodeMethod<T>,
    RW: AsyncWrite + Unpin,
{
    pub async fn send(&mut self, item: T) -> Result<(), SenderError<T, E>> {
        SinkExt::send(self, item).await
    }
}

impl<T, E, RW> Sink<T> for Sender<T, E, RW>
where
    T: 'static + fmt::Debug,
    E: 'static + EncodeMethod<T>,
    RW: AsyncWrite,
{
    type Error = SenderError<T, E>;

    fn start_send(self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.project().0.start_send(item).context(SenderSnafu)
    }

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_ready(cx)) {
            return Poll::Ready(Err(e).context(SenderSnafu));
        }

        Poll::Ready(Ok(()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_flush(cx)) {
            return Poll::Ready(Err(e).context(SenderSnafu));
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_close(cx)) {
            return Poll::Ready(Err(e).context(SenderSnafu));
        }

        Poll::Ready(Ok(()))
    }
}

#[pin_project::pin_project]
pub struct Receiver<T: fmt::Debug, E, RW = TcpSplit>(#[pin] pub(crate) channel::Channel<T, E, RW>);

impl<T: fmt::Debug, E, RW> From<broadcast::Receiver<T, E, RW>> for Receiver<T, E, RW> {
    fn from(receiver: broadcast::Receiver<T, E, RW>) -> Self {
        Receiver(receiver.0)
    }
}

impl<T: fmt::Debug, E, RW> From<channel::Channel<T, E, RW>> for Receiver<T, E, RW> {
    fn from(channel: channel::Channel<T, E, RW>) -> Self {
        Receiver(channel)
    }
}

impl<T, E, RW> Receiver<T, E, RW>
where
    T: fmt::Debug + Clone,
{
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, R, W> Receiver<T, E, RWSplit<R, W>>
where
    T: fmt::Debug,
{
    pub fn split(self) -> Result<(Self, Sender<T, E, RWSplit<R, W>>), channel::errors::SplitError> {
        self.0.split()
    }
}

impl<T: 'static + fmt::Debug + Clone, E: 'static + DecodeMethod<T>, RW: AsyncRead + Unpin>
    Receiver<T, E, RW>
{
    pub async fn recv(&mut self) -> Option<Result<T, ReceiverError<T, E>>> {
        self.next().await
    }
}

impl<T: 'static + fmt::Debug + Clone, E: 'static + DecodeMethod<T>, RW: AsyncRead> Stream
    for Receiver<T, E, RW>
{
    type Item = Result<T, ReceiverError<T, E>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match ready!(self.project().0.poll_next(cx)) {
            Some(Ok(item)) => Poll::Ready(Some(Ok(item))),
            Some(Err(e)) => Poll::Ready(Some(Err(e).context(ReceiverSnafu))),
            None => Poll::Ready(None),
        }
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(display("[SenderError] Failed to send item on mpsc::Sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderError<T, E>
    where
        T: 'static + fmt::Debug,
        E: 'static + EncodeMethod<T>,
        E::Error: 'static + fmt::Debug + std::error::Error,
    {
        source: channel::errors::ChannelSinkError<T, E>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[ReceiverError] Failed to receiver item on mpsc::Receiver"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverError<T, E>
    where
        T: 'static + fmt::Debug,
        E: 'static + DecodeMethod<T>,
        E::Error: 'static + fmt::Debug + std::error::Error,
    {
        source: channel::errors::ChannelStreamError<T, E>,
        backtrace: Backtrace,
    }
}
