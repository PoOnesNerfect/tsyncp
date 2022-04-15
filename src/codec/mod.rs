//! Contains adaptors from AsyncRead/AsyncWrite to Stream/Sink.
//!
//! For more information, check out [tokio_util::codec]
//!
//! [tokio_util::codec]: https://docs.rs/tokio-util/latest/tokio_util/codec/index.html

use bytes::{Bytes, BytesMut};

#[cfg(feature = "serde")]
use serde::{de::DeserializeOwned, Serialize};

pub mod length_delimited;

/// Type alias for [Framed] with our custom length delimited codec.
///
/// [Framed]: https://docs.rs/tokio-util/latest/tokio_util/codec/struct.Framed.html
pub type Framed<T, U = length_delimited::LengthDelimitedCodec> = tokio_util::codec::Framed<T, U>;

/// Trait for encoding methods which converts a given item into bytes.
pub trait EncodeMethod<T> {
    type Error: std::error::Error;

    fn encode(data: &T) -> Result<Bytes, Self::Error>;
}

/// Trait for decoding methods which converts given bytes into an item.
pub trait DecodeMethod<T> {
    type Error: std::error::Error;

    fn decode(bytes: BytesMut) -> Result<T, Self::Error>;
}

#[cfg(feature = "rkyv")]
pub use rkyv_mod::*;
#[cfg(feature = "rkyv")]
mod rkyv_mod {
    use super::*;
    use rkyv::{
        ser::{
            serializers::{
                AlignedSerializer, AllocScratch, AllocScratchError, CompositeSerializer,
                CompositeSerializerError, FallbackScratch, HeapScratch, SharedSerializeMap,
                SharedSerializeMapError,
            },
            Serializer,
        },
        AlignedVec, Fallible, Infallible,
    };

    pub struct RkyvCodec;

    impl<
            T: rkyv::Serialize<
                CompositeSerializer<
                    AlignedSerializer<AlignedVec>,
                    FallbackScratch<HeapScratch<256_usize>, AllocScratch>,
                    SharedSerializeMap,
                >,
            >,
        > EncodeMethod<T> for RkyvCodec
    {
        // type Error = CompositeSerializerError<Infallible, AllocScratchError>;
        type Error = CompositeSerializerError<
            std::convert::Infallible,
            AllocScratchError,
            SharedSerializeMapError,
        >;

        fn encode(data: &T) -> Result<Bytes, Self::Error> {
            rkyv::to_bytes::<_, 256>(data).map(|bytes| Bytes::from(bytes.to_vec()))
        }
    }

    impl<T: prost::Message + Default> DecodeMethod<T> for RkyvCodec {
        type Error = prost::DecodeError;

        fn decode(mut bytes: BytesMut) -> Result<T, Self::Error> {
            T::decode(&mut bytes)
        }
    }
}

#[cfg(feature = "protobuf")]
pub use protobuf::*;
#[cfg(feature = "protobuf")]
mod protobuf {
    use super::*;

    pub struct ProtobufCodec;

    impl<T: prost::Message> EncodeMethod<T> for ProtobufCodec {
        type Error = prost::EncodeError;

        fn encode(data: &T) -> Result<Bytes, Self::Error> {
            Ok(data.encode_to_vec().into())
        }
    }

    impl<T: prost::Message + Default> DecodeMethod<T> for ProtobufCodec {
        type Error = prost::DecodeError;

        fn decode(mut bytes: BytesMut) -> Result<T, Self::Error> {
            T::decode(&mut bytes)
        }
    }
}

#[cfg(feature = "json")]
pub use json::*;
#[cfg(feature = "json")]
mod json {
    use super::*;

    pub struct JsonCodec;

    impl<T: Serialize> EncodeMethod<T> for JsonCodec {
        type Error = serde_json::Error;

        fn encode(data: &T) -> Result<Bytes, Self::Error> {
            serde_json::to_vec(data).map(Into::into)
        }
    }

    impl<T: DeserializeOwned> DecodeMethod<T> for JsonCodec {
        type Error = serde_json::Error;

        fn decode(bytes: BytesMut) -> Result<T, Self::Error> {
            serde_json::from_slice(bytes.as_ref())
        }
    }
}
