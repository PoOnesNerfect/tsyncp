use crate::{
    channel,
    multi_channel::{self, accept::AcceptFuture, send::SendFuture},
    util::{
        accept::Accept,
        codec::{DecodeMethod, EncodeMethod},
        listener::WriteListener,
        tcp,
    },
};
use futures::Future;
use std::net::{SocketAddr, ToSocketAddrs};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
};

pub mod builder;

#[cfg(feature = "json")]
pub type JsonReceiver<T> = Receiver<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "json")]
pub type JsonSender<T, const N: usize = 0> = Sender<T, crate::util::codec::JsonCodec, N>;

#[cfg(feature = "protobuf")]
pub type ProtobufReceiver<T> = Receiver<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufSender<T, const N: usize = 0> = Sender<T, crate::util::codec::ProtobufCodec, N>;

#[cfg(feature = "rkyv")]
pub type RkyvReceiver<T> = Receiver<T, crate::util::codec::RkyvCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvSender<T, const N: usize = 0> = Sender<T, crate::util::codec::RkyvCodec, N>;

pub fn send_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::SenderBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E>>>,
> {
    builder::new_sender(local_addr)
}

pub fn recv_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::ReceiverBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::new_receiver(dest)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Receiver<T, E, S = tcp::OwnedReadHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Receiver<T, E, S> {
    fn from(c: channel::Channel<T, E, S>) -> Self {
        Self(c)
    }
}

impl<T, E, S> From<Receiver<T, E, S>> for channel::Channel<T, E, S> {
    fn from(c: Receiver<T, E, S>) -> Self {
        c.0
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

impl<T: Clone, E: DecodeMethod<T>, S: AsyncRead + Unpin> Receiver<T, E, S> {
    pub fn recv(&mut self) -> crate::channel::recv::RecvFuture<'_, T, E, S> {
        self.0.recv()
    }
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Sender<T, E, const N: usize = 0, L: Accept = WriteListener<TcpListener>>(
    #[pin] multi_channel::Channel<T, E, N, L>,
);

impl<T, E, const N: usize, L> From<multi_channel::Channel<T, E, N, L>> for Sender<T, E, N, L>
where
    L: Accept,
{
    fn from(c: multi_channel::Channel<T, E, N, L>) -> Self {
        Self(c)
    }
}

impl<T, E, const N: usize, L> From<Sender<T, E, N, L>> for multi_channel::Channel<T, E, N, L>
where
    L: Accept,
{
    fn from(c: Sender<T, E, N, L>) -> Self {
        c.0
    }
}

impl<T, E, const N: usize, L> Sender<T, E, N, L>
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

impl<T, E, const N: usize, L> Sender<T, E, N, L>
where
    L: Accept,
{
    pub fn accept(&mut self) -> AcceptFuture<'_, T, E, N, L> {
        self.0.accept()
    }
}

impl<T, E, const N: usize, L> Sender<T, E, N, L>
where
    T: Clone,
    E: EncodeMethod<T>,
    L: Accept,
    L::Output: AsyncWrite + Unpin,
{
    pub fn send(&mut self, item: T) -> SendFuture<'_, T, E, N, L> {
        self.0.send(item)
    }

    pub fn broadcast(&mut self, item: T) -> SendFuture<'_, T, E, N, L> {
        self.send(item)
    }
}
