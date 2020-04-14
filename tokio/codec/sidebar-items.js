initSidebarItems({"mod":[["length_delimited","Frame a stream of bytes based on a length prefix"]],"struct":[["BytesCodec","A simple `Codec` implementation that just ships bytes around."],["Framed","A unified `Stream` and `Sink` interface to an underlying I/O object, using the `Encoder` and `Decoder` traits to encode and decode frames."],["FramedParts","`FramedParts` contains an export of the data of a Framed transport. It can be used to construct a new `Framed` with a different codec. It contains all current buffers and the inner transport."],["FramedRead","A `Stream` of messages decoded from an `AsyncRead`."],["FramedWrite","A `Sink` of frames encoded to an `AsyncWrite`."],["LinesCodec","A simple `Codec` implementation that splits up data into lines."]],"trait":[["Decoder","Decoding of frames via buffers."],["Encoder","Trait of helper objects to write out messages as bytes, for use with `FramedWrite`."]]});