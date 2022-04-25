# tsyncp

### Synchronization primitives over TCP for message passing between services.

Major rust libraries such as [std] and [tokio] provide great synchronization primitives to be
used for message-passing between threads and tasks. However, there are not many libraries
that provide similar APIs that can be used over the network.

**Tsyncp** tries to fill the gap by providing the similar APIs for used over TCP. If you
have a project where it only has a few services running, and they need to pass some data to each other.
Instead of setting up a message-broker, you can use tsyncp to easily pass data between them.

**Tsyncp** also allows customizing different Serialization/Deserialization methods to encode/decode
data; currently, supported schemes straight from the library are _Json_, _Protobuf_, _Bincode_, _Speedy_, and _Rkyv_; however,
users can very easily implement their own [EncodeMethod] and [DecodeMethod].

_Warning: Tsyncp is not a message-broker nor it tries to be;
it's just a simple message-passer for simple and convenient use cases._

[std]: https://doc.rust-lang.org/stable/std/
[tokio]: https://docs.rs/tokio/latest/tokio/index.html
[encodemethod]: crate::util::codec::EncodeMethod
[decodemethod]: crate::util::codec::DecodeMethod
