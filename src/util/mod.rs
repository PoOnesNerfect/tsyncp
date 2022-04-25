use std::{any::type_name, fmt, marker::PhantomData};

pub mod codec;
pub mod frame_codec;
pub mod split;
pub mod stream_pool;

/// Type alias for [Framed] with our custom length delimited codec.
///
/// [Framed]: https://docs.rs/tokio-util/latest/tokio_util/codec/struct.Framed.html
pub type Framed<T, U = crate::util::frame_codec::VariedLengthDelimitedCodec> =
    tokio_util::codec::Framed<T, U>;

#[derive(Debug, Default, Clone, Copy)]
pub struct TcpStreamSettings {
    pub nodelay: Option<bool>,
    pub ttl: Option<u32>,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct TypeName<C>(&'static str, PhantomData<C>);

impl<C> fmt::Display for TypeName<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<C> snafu::GenerateImplicitData for TypeName<C> {
    fn generate() -> Self {
        Self(type_name::<C>(), PhantomData)
    }
}
