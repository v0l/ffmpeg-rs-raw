use crate::{Decoder, Demuxer, Encoder, Muxer, Scaler};

pub struct Transcoder {
    demuxer: Demuxer,
    decoder: Decoder,
    scaler: Scaler,
    encoder: Encoder,
    muxer: Muxer,
}
