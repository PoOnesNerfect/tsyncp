use super::{accept::Accept, split::Split};
use errors::*;
use snafu::{Backtrace, ResultExt};
use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

#[derive(Debug)]
pub struct ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    listener: L,
    capacity: usize,
    r_ret: Arc<
        Mutex<(
            VecDeque<(<<L as Accept>::Output as Split>::Left, SocketAddr)>,
            Option<Waker>,
        )>,
    >,
    w_ret: Arc<
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
    pub fn new(listener: L) -> Self {
        Self {
            listener,
            capacity: 1024,
            r_ret: Arc::new(Mutex::new((VecDeque::with_capacity(1024), None))),
            w_ret: Arc::new(Mutex::new((VecDeque::with_capacity(1024), None))),
        }
    }

    pub fn with_capacity(listener: L, capacity: usize) -> Self {
        Self {
            listener,
            capacity,
            r_ret: Arc::new(Mutex::new((VecDeque::with_capacity(capacity), None))),
            w_ret: Arc::new(Mutex::new((VecDeque::with_capacity(capacity), None))),
        }
    }

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
    pub fn poll_accept_r(
        &self,
        config: &L::Config,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<
        Result<(<<L as Accept>::Output as Split>::Left, SocketAddr), SplitAcceptError<L::Error>>,
    > {
        let mut r_ret = self.r_ret.lock().map_err(|_| SplitAcceptError::RMutex {
            backtrace: Backtrace::new(),
        })?;

        if !r_ret.0.is_empty() {
            return Poll::Ready(Ok(r_ret.0.pop_front().expect("element exists!")));
        }

        r_ret.1.replace(cx.waker().clone());

        match futures::ready!(self.listener.poll_accept(config, cx)) {
            Ok((s, a)) => {
                let (r, w) = s.split();

                let mut w_ret = self.w_ret.lock().map_err(|_| WMutexSnafu.build())?;

                if w_ret.0.len() >= self.capacity {
                    w_ret.0.pop_front();
                }

                w_ret.0.push_back((w, a));

                if w_ret.1.is_some() {
                    w_ret.1.take().unwrap().wake();
                }

                return Poll::Ready(Ok((r, a)));
            }
            Err(error) => return Poll::Ready(Err(error).context(AcceptingSnafu)),
        }
    }

    pub fn handle_abrupt_drop_r(&self) {
        if let Ok(mut w_ret) = self.w_ret.lock() {
            if w_ret.1.is_some() {
                w_ret.1.take().unwrap().wake();
            }
        }
    }

    pub fn poll_accept_w(
        &self,
        config: &L::Config,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<
        Result<(<<L as Accept>::Output as Split>::Right, SocketAddr), SplitAcceptError<L::Error>>,
    > {
        let mut w_ret = self.w_ret.lock().map_err(|_| SplitAcceptError::RMutex {
            backtrace: Backtrace::new(),
        })?;

        if !w_ret.0.is_empty() {
            return Poll::Ready(Ok(w_ret.0.pop_front().expect("element exists!")));
        }

        w_ret.1.replace(cx.waker().clone());

        match futures::ready!(self.listener.poll_accept(config, cx)) {
            Ok((s, a)) => {
                let (r, w) = s.split();

                let mut r_ret = self.r_ret.lock().map_err(|_| RMutexSnafu.build())?;

                if r_ret.0.len() >= self.capacity {
                    r_ret.0.pop_front();
                }

                r_ret.0.push_back((r, a));

                if r_ret.1.is_some() {
                    r_ret.1.take().unwrap().wake();
                }

                return Poll::Ready(Ok((w, a)));
            }
            Err(error) => return Poll::Ready(Err(error).context(AcceptingSnafu)),
        }
    }

    pub fn handle_abrupt_drop_w(&self) {
        if let Ok(mut r_ret) = self.r_ret.lock() {
            if r_ret.1.is_some() {
                r_ret.1.take().unwrap().wake();
            }
        }
    }
}

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
    type Error = SplitAcceptError<L::Error>;

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
    type Error = SplitAcceptError<L::Error>;

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
    type Error = UnsplitListenerError;

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

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SplitAcceptError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[SplitAcceptError] Failed to accept connection"))]
        Accepting { source: E, backtrace: Backtrace },
        #[snafu(display(
            "[SplitAcceptError] Failed to lock r_ret Mutex for access because it was poisoned"
        ))]
        RMutex { backtrace: Backtrace },
        #[snafu(display(
            "[SplitAcceptError] Failed to lock w_ret Mutex for access because it was poisoned"
        ))]
        WMutex { backtrace: Backtrace },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum UnsplitListenerError {
        #[snafu(display("[UnsplitListenerError] Failed try_unwrap on Arc reference of Listener"))]
        TryUnwrap { backtrace: Backtrace },
        #[snafu(display("[UnsplitListenerError] Given Arc pointers in unsplit are not the same"))]
        ArcPtrUnequal { backtrace: Backtrace },
    }
}
