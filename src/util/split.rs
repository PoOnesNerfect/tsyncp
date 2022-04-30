use super::{frame_codec::VariedLengthDelimitedCodec, tcp, Framed};
use tokio::net::TcpStream;
use tokio_util::codec::FramedParts;

pub trait Split: Sized {
    type Left;
    type Right;
    type Error: 'static + snafu::Error;

    fn split(self) -> (Self::Left, Self::Right);
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
