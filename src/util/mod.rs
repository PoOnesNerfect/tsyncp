use errors::*;
use frame_codec::VariedLengthDelimitedCodec;
use futures::ready;
use snafu::{Backtrace, ResultExt};
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio_util::codec::FramedParts;

pub mod codec;
pub mod frame_codec;
pub mod stream_pool;
pub mod tcp;

/// Type alias for [Framed] with our custom length delimited codec.
///
/// [Framed]: https://docs.rs/tokio-util/latest/tokio_util/codec/struct.Framed.html
pub type Framed<T, U = crate::util::frame_codec::VariedLengthDelimitedCodec> =
    tokio_util::codec::Framed<T, U>;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TcpStreamSettings {
    pub nodelay: Option<bool>,
    pub ttl: Option<u32>,
}

pub trait Accept {
    type Output: fmt::Debug;
    type Config: ConfigTrait;
    type Error: 'static + std::error::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>>;
}

pub trait ConfigTrait: fmt::Debug + PartialEq + Clone {}
impl<T: fmt::Debug + PartialEq + Clone> ConfigTrait for T {}

impl Accept for TcpListener {
    type Output = TcpStream;
    type Config = TcpStreamSettings;
    type Error = std::io::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>> {
        let (o, i) = match ready!(self.poll_accept(cx)) {
            Ok((o, i)) => (o, i),
            Err(error) => return Poll::Ready(Err(error)),
        };

        if let Some(nodelay) = config.nodelay {
            o.set_nodelay(nodelay)?;
        }

        if let Some(ttl) = config.ttl {
            o.set_ttl(ttl)?;
        }

        Poll::Ready(Ok((o, i)))
    }
}

#[derive(Debug)]
pub struct ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    listener: L,
    r_ret: Arc<
        Mutex<(
            Option<(<<L as Accept>::Output as Split>::Left, SocketAddr)>,
            Option<Waker>,
        )>,
    >,
    w_ret: Arc<
        Mutex<(
            Option<(<<L as Accept>::Output as Split>::Right, SocketAddr)>,
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
        Self {
            listener,
            r_ret: Arc::new(Mutex::new((None, None))),
            w_ret: Arc::new(Mutex::new((None, None))),
        }
    }
}

impl<L> Accept for ListenerWrapper<L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Output = L::Output;
    type Config = L::Config;
    type Error = SplitAcceptError<L::Error>;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>> {
        let res = ready!(self.listener.poll_accept(config, cx)).context(AcceptingSnafu);
        Poll::Ready(res)
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
        if r_ret.0.is_some() {
            return Poll::Ready(Ok(r_ret.0.take().expect("item is some")));
        }

        r_ret.1.replace(cx.waker().clone());

        match futures::ready!(self.listener.poll_accept(config, cx)) {
            Ok((s, addr)) => {
                let (r, w) = s.split();

                let mut w_ret = self.w_ret.lock().map_err(|_| SplitAcceptError::WMutex {
                    backtrace: Backtrace::new(),
                })?;
                w_ret.0.replace((w, addr));

                if w_ret.1.is_some() {
                    w_ret.1.take().expect("waker is some").wake();
                }

                return Poll::Ready(Ok((r, addr)));
            }
            Err(error) => return Poll::Ready(Err(error).context(AcceptingSnafu)),
        }
    }

    pub fn poll_accept_w(
        &self,
        config: &L::Config,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<
        Result<(<<L as Accept>::Output as Split>::Right, SocketAddr), SplitAcceptError<L::Error>>,
    > {
        let mut w_ret = self.w_ret.lock().map_err(|_| SplitAcceptError::WMutex {
            backtrace: Backtrace::new(),
        })?;
        if w_ret.0.is_some() {
            return Poll::Ready(Ok(w_ret.0.take().expect("item is some")));
        }

        w_ret.1.replace(cx.waker().clone());

        match futures::ready!(self.listener.poll_accept(config, cx)) {
            Ok((s, addr)) => {
                let (r, w) = s.split();

                let mut r_ret = self.r_ret.lock().map_err(|_| SplitAcceptError::RMutex {
                    backtrace: Backtrace::new(),
                })?;
                r_ret.0.replace((r, addr));

                if r_ret.1.is_some() {
                    r_ret.1.take().expect("waker is some").wake();
                }

                return Poll::Ready(Ok((w, addr)));
            }
            Err(error) => return Poll::Ready(Err(error).context(AcceptingSnafu)),
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
}

pub trait Split: Sized {
    type Left;
    type Right;
    type Error: 'static + std::error::Error;

    fn split(self) -> (Self::Left, Self::Right);
    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error>;
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

impl Split for TcpStream {
    type Left = tcp::OwnedReadHalf;
    type Right = tcp::OwnedWriteHalf;
    type Error = tcp::ReuniteError;

    fn split(self) -> (Self::Left, Self::Right) {
        let (r, w) = self.into_split();

        (r.into(), w.into())
    }

    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error> {
        tcp::reunite(left, right)
    }
}

impl<S: Split> Split for Framed<S> {
    type Left = Framed<S::Left>;
    type Right = Framed<S::Right>;
    type Error = S::Error;

    fn split(self) -> (Self::Left, Self::Right) {
        let FramedParts {
            io,
            codec,
            read_buf,
            write_buf,
            ..
        } = self.into_parts();

        let (r, w) = io.split();

        let mut r = FramedParts::new(r, codec);
        r.read_buf = read_buf;

        let mut w = FramedParts::new(w, VariedLengthDelimitedCodec::new());
        w.write_buf = write_buf;

        (Framed::from_parts(r), Framed::from_parts(w))
    }

    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error> {
        let FramedParts {
            io: r,
            codec,
            read_buf,
            ..
        } = left.into_parts();

        let FramedParts {
            io: w, write_buf, ..
        } = right.into_parts();

        let rw = S::unsplit(r, w)?;

        let mut rw = FramedParts::new(rw, codec);
        rw.read_buf = read_buf;
        rw.write_buf = write_buf;

        Ok(Framed::from_parts(rw))
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SplitError {
        #[snafu(display("[SplitError] Cannot split if a variant is not RWSplit::RW"))]
        Split,
        #[snafu(display("[SplitError] Cannot unsplit; Left variant must be RWSplit::R, and right variant must be RWSplit::W"))]
        Unsplit,
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SplitAcceptError<E>
    where
        E: 'static + std::error::Error,
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
