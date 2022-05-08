//! Contains [ListenerWrapper], [ReadListener] and [WriteListener] which allows splitting
//! generic `Listener` into `ReadListener` and `WriteListener`.
//!
//! In this library, you can split [multi_channel::Channel](crate::multi_channel::Channel)
//! into [mpsc::Receiver](crate::mpsc::Receiver) and [broadcast::Sender](crate::broadcast::Sender).
//! Since channels also include `Listener` (i.e. `TcpListener`) which accept incoming connections,
//! we also need to split its `Listener` into the Receiver half and Sender half.
//!
//! Structs in this module attempts to do just that.
//!
//! `ListenerWrapper<L>` is a wrapper around any type that implements [Accept](crate::util::Accept)
//! trait and `Accept::Output: Split`. `ListenerWrapper` has methods [poll_accept_r](ListenerWrapper::poll_accept_r)
//! and [poll_accept_w](ListenerWrapper::poll_accept_w); [poll_accept_r](ListenerWrapper::poll_accept_r) tries to accept
//! an incoming connection, splits the connection into `ReadHalf` and `WriteHalf`, then stores the
//! `WriteHalf` into write connections queue and returns the `ReadHalf`. [poll_accept_w](ListenerWrapper::poll_accept_w)
//! does the same thing but in the other direction.
//!
//! `ListenerWrapper<L>` splits into `ReadListener<L>` and `WriteListener<L>`.
//!
//! `ReadListener<L>` is a wrapper with an `Arc` around `ListenerWrapper<L>`, and uses
//! `poll_accept_r` as its `Accept` implementation. `WriteListener<L>` does the same but in the
//! other direction.
//!
//! With this implementation, [multi_channel::Channel](crate::multi_channel::Channel) can split along with its listener into
//! `Receiver` and `Sender` pair, with `Receiver` taking `ReadListener` as its listener, and
//! `Sender` taking `WriteListener` as its listener.

use super::{Accept, Split};
use errors::*;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

/// Wrapper around any type that implements [Accept](crate::util::Accept) and
/// its `Accept::Output: Split`.
///
/// This struct can be [split](crate::util::Split::split) into [ReadListener] and [WriteListener].
///
/// When this struct accepts a new connection, it splits the connection into `ReadHalf` and
/// `WriteHalf`, then queues them into read listener queue and write listener queue. `ReadListener`
/// will take connections from read listener queue, and `WriteListener` will take connections
/// from write listener queue.
///
/// This struct can queue up to 1024 accepted connections; when the queue of connections is full,
/// the oldest connection is popped from the queue and new incoming connection is pushed into the
/// queue.
#[derive(Debug)]
pub struct ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    // inner listener.
    listener: L,
    // total number of listener that can be queued.
    capacity: usize,
    // queue where the read listener can take a new connection from.
    reader_queue: Arc<
        Mutex<(
            VecDeque<(<<L as Accept>::Output as Split>::Left, SocketAddr)>,
            Option<Waker>,
        )>,
    >,
    // queue where the write listener can take a new connection from.
    writer_queue: Arc<
        Mutex<(
            VecDeque<(<<L as Accept>::Output as Split>::Right, SocketAddr)>,
            Option<Waker>,
        )>,
    >,
}

impl<L> ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    /// Returns new instance of `ListenerWrapper` with default limit of 1024.
    pub fn new(listener: L) -> Self {
        let capacity = 1024;

        Self {
            listener,
            capacity,
            reader_queue: Arc::new(Mutex::new((VecDeque::with_capacity(capacity / 8), None))),
            writer_queue: Arc::new(Mutex::new((VecDeque::with_capacity(capacity / 8), None))),
        }
    }

    /// Returns the inner listener.
    pub fn into_inner(self) -> L {
        self.listener
    }
}

impl<L> From<L> for ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    fn from(listener: L) -> Self {
        Self::new(listener)
    }
}

impl<L> ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    /// Polls the inner listener; if new connection is received, it splits the connection into
    /// `ReadHalf` and `WriteHalf`, queues the `WriteHalf` and returns the `ReadHalf`.
    ///
    /// If there is a waker stored for the `WriteListener`, it wakes up the waker so that it can
    /// take the `WriteHalf`.
    pub fn poll_accept_r(
        &self,
        config: &L::Config,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(<<L as Accept>::Output as Split>::Left, SocketAddr), L::Error>> {
        let mut reader_queue = self.reader_queue.lock();

        // First, try to see if there is a connection in the queue.
        if !reader_queue.0.is_empty() {
            return Poll::Ready(Ok(reader_queue.0.pop_front().expect("element exists!")));
        }

        // Save the waker so that write listener can wake it once new connection is received.
        reader_queue.1.replace(cx.waker().clone());

        // try poll accept
        match futures::ready!(self.listener.poll_accept(config, cx)) {
            Ok((s, a)) => {
                // split the accepted connection into read half and write half.
                let (r, w) = s.split();

                let mut writer_queue = self.writer_queue.lock();

                // if the writer queue is at capacity, pop the oldest connection.
                if writer_queue.0.len() >= self.capacity {
                    writer_queue.0.pop_front();
                }

                // queue the new connection.
                writer_queue.0.push_back((w, a));

                // wake the write listener if exists.
                if writer_queue.1.is_some() {
                    writer_queue.1.take().unwrap().wake();
                }

                // return the read half of the stream.
                return Poll::Ready(Ok((r, a)));
            }
            Err(error) => return Poll::Ready(Err(error)),
        }
    }

    /// Handles the case where the read listener's accept future is cancelled when it did not yet
    /// accept a new connection. This is necessary because `TcpListener`'s accept future only wakes
    /// the last stored waker. So, if the read listener polled it last, and stops polling, the
    /// listener will try to wake the read listener and leave the write listener hanging.
    ///
    /// It wakes up the WriteListener's waker, so that it can continue polling.
    pub fn handle_abrupt_drop_r(&self) {
        let mut writer_queue = self.writer_queue.lock();

        // if there is a writer listener listening to new connections,
        // wake it up so that it can poll the inner listener again.
        if writer_queue.1.is_some() {
            writer_queue.1.take().unwrap().wake();
        }
    }

    /// Polls the inner listener; if new connection is received, it splits the connection into
    /// `ReadHalf` and `WriteHalf`, queues the `ReadHalf` and returns the `WriteHalf`.
    ///
    /// If there is a waker stored for the `ReadListener`, it wakes up the waker so that it can
    /// take the `ReadHalf`.
    pub fn poll_accept_w(
        &self,
        config: &L::Config,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(<<L as Accept>::Output as Split>::Right, SocketAddr), L::Error>> {
        let mut writer_queue = self.writer_queue.lock();

        // First, try to see if there is a connection in the queue.
        if !writer_queue.0.is_empty() {
            return Poll::Ready(Ok(writer_queue.0.pop_front().expect("element exists!")));
        }

        // Save the waker so that read listener can wake it once new connection is received.
        writer_queue.1.replace(cx.waker().clone());

        // try poll accept
        match futures::ready!(self.listener.poll_accept(config, cx)) {
            Ok((s, a)) => {
                // split the accepted connection into read half and write half.
                let (r, w) = s.split();

                let mut reader_queue = self.reader_queue.lock();

                // if the reader queue is at capacity, pop the oldest connection.
                if reader_queue.0.len() >= self.capacity {
                    reader_queue.0.pop_front();
                }

                // queue the new connection.
                reader_queue.0.push_back((r, a));

                // wake the read listener if exists.
                if reader_queue.1.is_some() {
                    reader_queue.1.take().unwrap().wake();
                }

                // return the write half of the stream.
                return Poll::Ready(Ok((w, a)));
            }
            Err(error) => return Poll::Ready(Err(error)),
        }
    }

    /// Handles the case where the write listener's accept future is cancelled when it did not yet
    /// accept a new connection. This is necessary because `TcpListener`'s accept future only wakes
    /// the last stored waker. So, if the write listener polled it last, and stops polling, the
    /// listener will try to wake the write listener and leave the read listener hanging.
    ///
    /// It wakes up the WriteListener's waker, so that it can continue polling.
    pub fn handle_abrupt_drop_w(&self) {
        let mut reader_queue = self.reader_queue.lock();

        // if there is a read listener listening to new connections,
        // wake it up so that it can poll the inner listener again.
        if reader_queue.1.is_some() {
            reader_queue.1.take().unwrap().wake();
        }
    }
}

/// Wrapper around `ListenerWrapper` which accepts the `ReadHalf` of a new incoming connection.
///
/// This struct wraps `ListenerWrapper` with an [Arc](std::sync::Arc),
/// which is shared with [WriteListener] when it is split from `ListenerWrapper`.
///
/// This struct implements `Accept`, which accepts and returns the `ReadHalf` of a connection, and
/// queues the `WriteHalf` into the write listener's queue.
#[derive(Debug)]
pub struct ReadListener<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    listener: Arc<ListenerWrapper<L>>,
}

impl<L> Accept for ReadListener<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Output = <<L as Accept>::Output as Split>::Left;
    type Config = L::Config;
    type Error = L::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>> {
        self.listener.poll_accept_r(config, cx)
    }

    fn handle_abrupt_drop(&self) {
        self.listener.listener.handle_abrupt_drop();
        self.listener.handle_abrupt_drop_r()
    }
}

/// Wrapper around `ListenerWrapper` which accepts the `WriteHalf` of a new incoming connection.
///
/// This struct wraps `ListenerWrapper` with an [Arc](std::sync::Arc),
/// which is shared with [ReadListener] when it is split from `ListenerWrapper`.
///
/// This struct implements `Accept`, which accepts and returns the `WriteHalf` of a connection, and
/// queues the `ReadHalf` into the read listener's queue.
#[derive(Debug)]
pub struct WriteListener<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    listener: Arc<ListenerWrapper<L>>,
}

impl<L> Accept for WriteListener<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Output = <<L as Accept>::Output as Split>::Right;
    type Config = L::Config;
    type Error = L::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>> {
        self.listener.poll_accept_w(config, cx)
    }

    fn handle_abrupt_drop(&self) {
        self.listener.listener.handle_abrupt_drop();
        self.listener.handle_abrupt_drop_w()
    }
}

impl<L> Split for ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Left = ReadListener<L>;
    type Right = WriteListener<L>;
    type Error = UnsplitError;

    fn split(self) -> (Self::Left, Self::Right) {
        let a = Arc::new(self);
        (
            ReadListener {
                listener: Arc::clone(&a),
            },
            WriteListener { listener: a },
        )
    }

    fn unsplit(left: ReadListener<L>, right: WriteListener<L>) -> Result<Self, Self::Error> {
        if Arc::ptr_eq(&left.listener, &right.listener) {
            drop(right);

            // This unwrap cannot fail as the api does not allow creating more than two Arcs,
            // and we just dropped the other half.
            if let Ok(listener) = Arc::try_unwrap(left.listener) {
                Ok(listener)
            } else {
                TryUnwrapSnafu.fail()
            }
        } else {
            ArcPtrUnequalSnafu.fail()
        }
    }
}

/// Contains listener errors.
pub mod errors {
    use snafu::{Backtrace, Snafu};

    /// Error returned while unsplitting `ReadListener` and `WriteListener`.
    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum UnsplitError {
        #[snafu(display("[UnsplitError] Failed try_unwrap on Arc reference of Listener"))]
        TryUnwrap { backtrace: Backtrace },
        #[snafu(display("[UnsplitError] Provided Arc pointers in unsplit are not the same"))]
        ArcPtrUnequal { backtrace: Backtrace },
    }
}
