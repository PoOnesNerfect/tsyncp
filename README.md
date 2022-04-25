# tsyncp

### Synchronization primitives over TCP for message passing.

Major rust libraries such as [std] and [tokio] provide great synchronization primitives to be
used for message-passing between threads and tasks. However, there are not many libraries
that provide similar APIs that can be used over the network.

**Tsyncp** tries to fill the gap by providing the similar APIs (mpsc, broadcast, barrier, etc) over TCP. If you
have a project where it only has a few services running, and they need to pass some data to each other;
instead of setting up a message-broker service, you can use tsyncp to easily pass data between them.

**Tsyncp** also allows customizing different Serialization/Deserialization methods to encode/decode
data; currently, supported schemes straight from the library are _Json_, _Protobuf_, ~_Bincode_~, ~_Speedy_~, and ~_Rkyv_~; however,
users can very easily implement their own [EncodeMethod] and [DecodeMethod].

***Note***: tsyncp is built on [tokio]; and thus may not be compatible with other async runtimes like async-std.

_Warning: Tsyncp is not a message-broker nor it tries to be;
it's just a message-passing library for simple and convenient use cases._

_Warning: Tsyncp is still WIP! It's usable, but still needs features implemented, extensive testing, documentations, and examples._

[std]: https://doc.rust-lang.org/stable/std/
[tokio]: https://docs.rs/tokio/latest/tokio/index.html
[encodemethod]: https://docs.rs/tsyncp/latest/tsyncp/util/codec/trait.EncodeMethod.html
[decodemethod]: https://docs.rs/tsyncp/latest/tsyncp/util/codec/trait.DecodeMethod.html
