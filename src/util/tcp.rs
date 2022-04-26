use std::io;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{tcp, TcpStream};

pub use tokio::net::tcp::ReuniteError;

pub(crate) fn reunite(
    read: OwnedReadHalf,
    mut write: OwnedWriteHalf,
) -> Result<TcpStream, ReuniteError> {
    let w_inner = write.take_inner();

    // don't need wrapper anymore
    drop(write);

    // reunite will call .forget() on tcp::OwnedWriteHalf
    read.inner.reunite(w_inner)
}

#[derive(Debug)]
pub struct OwnedReadHalf {
    inner: tcp::OwnedReadHalf,
}

impl AsyncRead for OwnedReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl From<tcp::OwnedReadHalf> for OwnedReadHalf {
    fn from(r: tcp::OwnedReadHalf) -> Self {
        Self { inner: r }
    }
}

impl Deref for OwnedReadHalf {
    type Target = tcp::OwnedReadHalf;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for OwnedReadHalf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// Light wrapper around tokio's OwnedWriteHalf to stop it from shutting down TCP stream with the
/// it drops. Option around the inner struct is to take it out when dropping, since custom Drop
/// implementation does not allow taking the inner object out of the struct.
#[derive(Debug)]
pub struct OwnedWriteHalf {
    inner: Option<tcp::OwnedWriteHalf>,
    should_forget: bool,
}

impl OwnedWriteHalf {
    // Should be only used before dropping the struct.
    fn take_inner(&mut self) -> tcp::OwnedWriteHalf {
        let inner = self.inner.take().expect("should exist");

        self.should_forget = false;

        inner
    }
}

impl From<tcp::OwnedWriteHalf> for OwnedWriteHalf {
    fn from(w: tcp::OwnedWriteHalf) -> Self {
        Self {
            inner: Some(w),
            should_forget: true,
        }
    }
}

impl AsyncWrite for OwnedWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let inner: &mut tcp::OwnedWriteHalf = self.deref_mut();
        Pin::new(inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let inner: &mut tcp::OwnedWriteHalf = self.deref_mut();
        Pin::new(inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        let inner: &tcp::OwnedWriteHalf = self.deref();
        inner.is_write_vectored()
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        // tcp flush is a no-op
        Poll::Ready(Ok(()))
    }

    // `poll_shutdown` on a write half shutdowns the stream in the "write" direction.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner: &mut tcp::OwnedWriteHalf = self.deref_mut();
        Pin::new(inner).poll_shutdown(cx)
    }
}

impl Drop for OwnedWriteHalf {
    fn drop(&mut self) {
        if self.should_forget {
            let inner = self.take_inner();
            inner.forget();
        }
    }
}

impl Deref for OwnedWriteHalf {
    type Target = tcp::OwnedWriteHalf;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().expect("Should exist")
    }
}

impl DerefMut for OwnedWriteHalf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().expect("Should exist")
    }
}
