use bytes::{Buf, BufMut, Bytes};
use tonic::Status;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};

/// Raw protobuf message codec. Tonic still owns the five-byte gRPC envelope.
#[derive(Clone, Copy, Debug, Default)]
pub struct RawCodec;

#[derive(Clone, Copy, Debug, Default)]
pub struct RawEncoder;

#[derive(Clone, Copy, Debug, Default)]
pub struct RawDecoder;

impl Codec for RawCodec {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = RawEncoder;
    type Decoder = RawDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        RawEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        RawDecoder
    }
}

impl Encoder for RawEncoder {
    type Item = Bytes;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, destination: &mut EncodeBuf<'_>) -> Result<(), Status> {
        destination.reserve(item.len());
        destination.put_slice(&item);
        Ok(())
    }
}

impl Decoder for RawDecoder {
    type Item = Bytes;
    type Error = Status;

    fn decode(&mut self, source: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Status> {
        let remaining = source.remaining();
        Ok(Some(source.copy_to_bytes(remaining)))
    }
}
