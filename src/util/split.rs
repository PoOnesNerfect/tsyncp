use super::TcpStreamSettings;
use super::{frame_codec::VariedLengthDelimitedCodec, Framed};
use errors::*;
use snafu::{Backtrace, ResultExt, Snafu};
use std::{fmt, io, net::SocketAddr};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{tcp, TcpListener},
};
use tokio_util::codec::FramedParts;

pub type TcpSplit = RWSplit<tcp::OwnedReadHalf, tcp::OwnedWriteHalf>;

pub fn split_framed<R, W>(
    framed: Framed<RWSplit<R, W>>,
) -> Result<(Framed<RWSplit<R, W>>, Framed<RWSplit<R, W>>), SplitError> {
    let FramedParts {
        io,
        codec,
        read_buf,
        write_buf,
        ..
    } = framed.into_parts();
    let (r, w) = io.split()?;

    let mut r = FramedParts::new(r, codec);
    r.read_buf = read_buf;

    let mut w = FramedParts::new(w, VariedLengthDelimitedCodec::new());
    r.write_buf = write_buf;

    Ok((Framed::from_parts(r), Framed::from_parts(w)))
}

pub fn unsplit_framed<R, W>(
    r: Framed<RWSplit<R, W>>,
    w: Framed<RWSplit<R, W>>,
) -> Result<Framed<RWSplit<R, W>>, SplitError> {
    let FramedParts {
        io: r,
        codec,
        read_buf,
        ..
    } = r.into_parts();

    let FramedParts {
        io: w, write_buf, ..
    } = w.into_parts();

    let rw = RWSplit::unsplit(r, w)?;

    let mut rw = FramedParts::new(rw, codec);
    rw.read_buf = read_buf;
    rw.write_buf = write_buf;

    Ok(Framed::from_parts(rw))
}

#[derive(Debug)]
#[pin_project::pin_project(project = ListenerSplitProj)]
pub enum ListenerSplit<L = TcpListener, T = TcpSplit> {
    Listener {
        listener: L,
    },
    Sender {
        listener: L,
        sender: tokio::sync::mpsc::UnboundedSender<(T, SocketAddr)>,
        is_readhalf: bool,
    },
    Receiver {
        receiver: tokio::sync::mpsc::UnboundedReceiver<(T, SocketAddr)>,
    },
}

impl<L, T> From<L> for ListenerSplit<L, T> {
    fn from(l: L) -> Self {
        Self::Listener { listener: l }
    }
}

impl ListenerSplit<TcpListener, TcpSplit> {
    pub fn is_listener(&self) -> bool {
        matches!(self, Self::Listener { .. })
    }

    pub fn is_sender(&self) -> bool {
        matches!(self, Self::Sender { .. })
    }

    pub fn is_receiver(&self) -> bool {
        matches!(self, Self::Receiver { .. })
    }

    pub fn split(self, is_readhalf: bool) -> Result<(Self, Self), ListenerSplitError<TcpSplit>> {
        let listener = if let Self::Listener { listener } = self {
            listener
        } else {
            return ListenerSplitSnafu.fail();
        };

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = Self::Sender {
            listener,
            sender: tx,
            is_readhalf,
        };
        let receiver = Self::Receiver { receiver: rx };
        Ok((sender, receiver))
    }

    pub async fn accept(
        &mut self,
        tcp_stream_settings: TcpStreamSettings,
    ) -> Result<(TcpSplit, SocketAddr), ListenerSplitError<TcpSplit>> {
        match self {
            Self::Listener { listener } => {
                let (stream, addr) = listener.accept().await.context(AcceptingSnafu)?;

                if let Some(nodelay) = tcp_stream_settings.nodelay {
                    stream
                        .set_nodelay(nodelay)
                        .context(SetNodelaySnafu { addr })?;
                }

                if let Some(ttl) = tcp_stream_settings.ttl {
                    stream.set_ttl(ttl).context(SetTtlSnafu { addr })?;
                }

                let (r, w) = stream.into_split();
                return Ok((TcpSplit::RW(r, w), addr));
            }
            Self::Sender {
                listener,
                sender,
                is_readhalf,
            } => {
                let (stream, addr) = listener.accept().await.context(AcceptingSnafu)?;

                if let Some(nodelay) = tcp_stream_settings.nodelay {
                    stream
                        .set_nodelay(nodelay)
                        .context(SetNodelaySnafu { addr })?;
                }

                if let Some(ttl) = tcp_stream_settings.ttl {
                    stream.set_ttl(ttl).context(SetTtlSnafu { addr })?;
                }

                let (r, w) = stream.into_split();
                let (returning, sending) = if *is_readhalf {
                    ((TcpSplit::R(r), addr), (TcpSplit::W(w), addr))
                } else {
                    ((TcpSplit::W(w), addr), (TcpSplit::R(r), addr))
                };
                sender.send(sending).context(SendingSnafu)?;
                return Ok(returning);
            }
            Self::Receiver { receiver } => {
                if let Some(w) = receiver.recv().await {
                    return Ok(w);
                } else {
                    ReceivingSnafu.fail()
                }
            }
        }
    }
}

#[derive(Debug, Snafu)]
pub enum ListenerSplitError<T>
where
    T: 'static + fmt::Debug,
{
    #[snafu(display("[ListenerSplitError] Failed to accept a connection"))]
    Accepting {
        source: std::io::Error,
        backtrace: Backtrace,
    },
    #[snafu(display("[ListenerSplitError] Failed to set nodelay for {addr}"))]
    SetNodelay {
        addr: SocketAddr,
        /// source IO Error
        source: io::Error,
        backtrace: Backtrace,
    },
    #[snafu(display("[ListenerSplitError] Failed to set ttl for {addr}"))]
    SetTtl {
        addr: SocketAddr,
        /// source IO Error
        source: io::Error,
        backtrace: Backtrace,
    },
    #[snafu(display("[ListenerSplitError] Failed to send an accepted connection"))]
    Sending {
        source: tokio::sync::mpsc::error::SendError<(T, SocketAddr)>,
        backtrace: Backtrace,
    },
    #[snafu(display("[ListenerSplitError] Listener half dropped"))]
    Receiving,
    #[snafu(display("[ListenerSplitError] Cannot split listener unless it's a Listener variant"))]
    ListenerSplit,
}

#[derive(Debug)]
#[pin_project::pin_project(project = RWSplitProj)]
pub enum RWSplit<R, W> {
    RW(#[pin] R, #[pin] W),
    R(#[pin] R),
    W(#[pin] W),
}

impl<R, W> RWSplit<R, W> {
    pub fn split(self) -> Result<(Self, Self), SplitError> {
        if let Self::RW(r, w) = self {
            Ok((Self::R(r), Self::W(w)))
        } else {
            Err(SplitError::Split)
        }
    }

    pub fn unsplit(r: Self, w: Self) -> Result<Self, SplitError> {
        if let (Self::R(r), Self::W(w)) = (r, w) {
            Ok(Self::RW(r, w))
        } else {
            Err(SplitError::Unsplit)
        }
    }
}

impl<R: AsyncRead, W> AsyncRead for RWSplit<R, W> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            RWSplitProj::RW(r, _) => r.poll_read(cx, buf),
            RWSplitProj::R(r) => r.poll_read(cx, buf),
            RWSplitProj::W(_) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Called poll_read on write_half",
            ))),
        }
    }
}

impl<R, W: AsyncWrite> AsyncWrite for RWSplit<R, W> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        match self.project() {
            RWSplitProj::RW(_, w) => w.poll_write(cx, buf),
            RWSplitProj::W(w) => w.poll_write(cx, buf),
            RWSplitProj::R(_) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Called poll_write on write_half",
            ))),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match self.project() {
            RWSplitProj::RW(_, w) => w.poll_flush(cx),
            RWSplitProj::W(w) => w.poll_flush(cx),
            RWSplitProj::R(_) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Called poll_flush on write_half",
            ))),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match self.project() {
            RWSplitProj::RW(_, w) => w.poll_shutdown(cx),
            RWSplitProj::W(w) => w.poll_shutdown(cx),
            RWSplitProj::R(_) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Called poll_shutdown on write_half",
            ))),
        }
    }

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        match self.project() {
            RWSplitProj::RW(_, w) => w.poll_write_vectored(cx, bufs),
            RWSplitProj::W(w) => w.poll_write_vectored(cx, bufs),
            RWSplitProj::R(_) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Called poll_write_vectored on write_half",
            ))),
        }
    }
}

pub mod errors {
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SplitError {
        #[snafu(display("[SplitError] Cannot split if a variant is not RWSplit::RW"))]
        Split,
        #[snafu(display("[SplitError] Cannot unsplit; Left variant must be RWSplit::R, and right variant must be RWSplit::W"))]
        Unsplit,
    }
}
