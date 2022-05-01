use crate::channel;
use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::tcp;
use futures::Future;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
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
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::sender_to(dest)
}

pub fn send_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::SenderOnBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::sender_on(local_addr)
}

pub fn recv_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::ReceiverToBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::receiver_to(dest)
}

pub fn recv_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ReceiverOnBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::receiver_on(local_addr)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Sender<T, E, S = tcp::OwnedWriteHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Sender<T, E, S> {
    fn from(channel: channel::Channel<T, E, S>) -> Self {
        Sender(channel)
    }
}

impl<T, E, S> From<Sender<T, E, S>> for channel::Channel<T, E, S> {
    fn from(channel: Sender<T, E, S>) -> Self {
        channel.0
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
pub struct Receiver<T, E, S = tcp::OwnedReadHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Receiver<T, E, S> {
    fn from(channel: channel::Channel<T, E, S>) -> Self {
        Receiver(channel)
    }
}

impl<T, E, S> From<Receiver<T, E, S>> for channel::Channel<T, E, S> {
    fn from(channel: Receiver<T, E, S>) -> Self {
        channel.0
    }
}

impl<T, E, S> Receiver<T, E, S> {
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, S> Receiver<T, E, S>
where
    E: DecodeMethod<T>,
    S: AsyncRead + Unpin,
{
    pub fn recv(&mut self) -> channel::recv::RecvFuture<'_, T, E, S> {
        self.0.recv()
    }
}
