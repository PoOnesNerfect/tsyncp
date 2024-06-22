use super::{frame_codec::VariedLengthDelimitedCodec, tcp, Framed};
use tokio::net::TcpStream;
use tokio_util::codec::FramedParts;

/// Provides abstraction over splitting an object into two different types.
///
/// This trait is useful for splitting a stream (i.e. TcpStream) into ReadHalf and WriteHalf,
/// and split a listener (i.e. TcpListener) into `ReadListener` and `WriteListener`.
///
/// These split structs are used when splitting a channel into `Receiver` and `Sender` pair.
pub trait Split: Sized {
    /// Left half of split item
    type Left;
    /// Right half of split item
    type Right;
    /// Error returned by associated method `unsplit(_, _)`.
    type Error: 'static + std::error::Error;

    /// Split self into left and right halfs.
    fn split(self) -> (Self::Left, Self::Right);

    /// Unite left half and right half into `Self` again.
    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error>;
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
