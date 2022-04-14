//! Contains adaptors from AsyncRead/AsyncWrite to Stream/Sink.
//!
//! For more information, check out [tokio_util::codec]
//!
//! [tokio_util::codec]: https://docs.rs/tokio-util/latest/tokio_util/codec/index.html

use bytes::Bytes;

#[cfg(feature = "serde")]
use serde::{de::DeserializeOwned, Serialize};

pub mod length_delimited;
pub use length_delimited::LengthDelimitedCodec;

/// Type alias for `Framed` with length delimited codec.
pub type Framed<T, U = length_delimited::LengthDelimitedCodec> = tokio_util::codec::Framed<T, U>;

pub trait FrameSerialize<T> {
    type Error: std::error::Error;

    fn serialize_frame(data: &T) -> Result<Bytes, Self::Error>;
}

/// Generic encoding/decoding scheme for turning a frame of bytes into an item.
pub trait FrameDeserialize<T> {
    type Error: std::error::Error;

    fn deserialize_frame(bytes: Bytes) -> Result<T, Self::Error>;
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

    pub struct RkyvFrame;

    impl<
            T: rkyv::Serialize<
                CompositeSerializer<
                    AlignedSerializer<AlignedVec>,
                    FallbackScratch<HeapScratch<256_usize>, AllocScratch>,
                    SharedSerializeMap,
                >,
            >,
        > FrameSerialize<T> for RkyvFrame
    {
        // type Error = CompositeSerializerError<Infallible, AllocScratchError>;
        type Error = CompositeSerializerError<
            std::convert::Infallible,
            AllocScratchError,
            SharedSerializeMapError,
        >;

        fn serialize_frame(data: &T) -> Result<Bytes, Self::Error> {
            rkyv::to_bytes::<_, 256>(data).map(|bytes| Bytes::from(bytes.to_vec()))
        }
    }

    impl<T: prost::Message + Default> FrameDeserialize<T> for RkyvFrame {
        type Error = prost::DecodeError;

        fn deserialize_frame(mut bytes: Bytes) -> Result<T, Self::Error> {
            T::decode(&mut bytes)
        }
    }
}

#[cfg(feature = "protobuf")]
pub use protobuf::*;
#[cfg(feature = "protobuf")]
mod protobuf {
    use super::*;

    pub struct ProtobufFrame;

    impl<T: prost::Message> FrameSerialize<T> for ProtobufFrame {
        type Error = prost::EncodeError;

        fn serialize_frame(data: &T) -> Result<Bytes, Self::Error> {
            let mut bytes = bytes::BytesMut::new();

            data.encode(&mut bytes)?;

            Ok(bytes.into())
        }
    }

    impl<T: prost::Message + Default> FrameDeserialize<T> for ProtobufFrame {
        type Error = prost::DecodeError;

        fn deserialize_frame(mut bytes: Bytes) -> Result<T, Self::Error> {
            T::decode(&mut bytes)
        }
    }
}

#[cfg(feature = "json")]
pub use json::*;
#[cfg(feature = "json")]
mod json {
    use super::*;

    pub struct JsonFrame;

    impl<T: Serialize> FrameSerialize<T> for JsonFrame {
        type Error = serde_json::Error;

        fn serialize_frame(data: &T) -> Result<Bytes, Self::Error> {
            serde_json::to_vec(data).map(Into::into)
        }
    }

    impl<T: DeserializeOwned> FrameDeserialize<T> for JsonFrame {
        type Error = serde_json::Error;

        fn deserialize_frame(bytes: Bytes) -> Result<T, Self::Error> {
            serde_json::from_slice(bytes.as_ref())
        }
    }
}
