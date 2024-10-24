use crate::{options_to_dict, StreamInfoChannel};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr;

use anyhow::Error;
use ffmpeg_sys_the_third::AVPictureType::AV_PICTURE_TYPE_NONE;
use ffmpeg_sys_the_third::{
    av_buffer_alloc, av_frame_alloc, avcodec_alloc_context3, avcodec_find_decoder,
    avcodec_free_context, avcodec_get_name, avcodec_open2, avcodec_parameters_to_context,
    avcodec_receive_frame, avcodec_send_packet, AVCodec, AVCodecContext, AVFrame, AVMediaType,
    AVPacket, AVStream, AVERROR, AVERROR_EOF,
};
use libc::memcpy;

pub struct DecoderCodecContext {
    pub context: *mut AVCodecContext,
    pub codec: *const AVCodec,
}

impl DecoderCodecContext {
    /// Set [AVCodecContext] options
    pub fn set_opt(&mut self, options: HashMap<String, String>) -> Result<(), Error> {
        crate::set_opts(self.context as *mut libc::c_void, options)
    }

    pub fn list_opts(&self) -> Result<Vec<String>, Error> {
        crate::list_opts(self.context as *mut libc::c_void)
    }
}

impl Drop for DecoderCodecContext {
    fn drop(&mut self) {
        unsafe {
            avcodec_free_context(&mut self.context);
            self.codec = ptr::null_mut();
            self.context = ptr::null_mut();
        }
    }
}

unsafe impl Send for DecoderCodecContext {}
unsafe impl Sync for DecoderCodecContext {}

pub struct Decoder {
    codecs: HashMap<i32, DecoderCodecContext>,
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            codecs: HashMap::new(),
        }
    }

    /// Set up a decoder for a given channel
    pub fn setup_decoder(
        &mut self,
        channel: &StreamInfoChannel,
        options: Option<HashMap<String, String>>,
    ) -> Result<&mut DecoderCodecContext, Error> {
        unsafe { self.setup_decoder_for_stream(channel.stream, options) }
    }

    /// Set up a decoder from an [AVStream]
    pub unsafe fn setup_decoder_for_stream(
        &mut self,
        stream: *mut AVStream,
        options: Option<HashMap<String, String>>,
    ) -> Result<&mut DecoderCodecContext, Error> {
        if stream.is_null() {
            anyhow::bail!("stream is null");
        }

        let codec_par = (*stream).codecpar;
        assert_ne!(
            codec_par,
            ptr::null_mut(),
            "Codec parameters are missing from stream"
        );

        if let std::collections::hash_map::Entry::Vacant(e) = self.codecs.entry((*stream).index) {
            let codec = avcodec_find_decoder((*codec_par).codec_id);
            if codec.is_null() {
                anyhow::bail!(
                    "Failed to find codec: {}",
                    CStr::from_ptr(avcodec_get_name((*codec_par).codec_id)).to_str()?
                )
            }
            let context = avcodec_alloc_context3(codec);
            if context.is_null() {
                anyhow::bail!("Failed to alloc context")
            }
            if avcodec_parameters_to_context(context, (*stream).codecpar) != 0 {
                anyhow::bail!("Failed to copy codec parameters to context")
            }
            let mut dict = if let Some(options) = options {
                options_to_dict(options)?
            } else {
                ptr::null_mut()
            };

            if avcodec_open2(context, codec, &mut dict) < 0 {
                anyhow::bail!("Failed to open codec")
            }
            Ok(e.insert(DecoderCodecContext { context, codec }))
        } else {
            anyhow::bail!("Decoder already setup");
        }
    }

    pub unsafe fn decode_pkt(
        &mut self,
        pkt: *mut AVPacket,
        stream: *mut AVStream,
    ) -> Result<Vec<(*mut AVFrame, *mut AVStream)>, Error> {
        let stream_index = (*pkt).stream_index;
        assert_eq!(
            stream_index,
            (*stream).index,
            "Passed stream reference does not match stream_index of packet"
        );

        if let Some(ctx) = self.codecs.get_mut(&stream_index) {
            // subtitles don't need decoding, create a frame from the pkt data
            if (*ctx.codec).type_ == AVMediaType::AVMEDIA_TYPE_SUBTITLE {
                let frame = av_frame_alloc();
                (*frame).pts = (*pkt).pts;
                (*frame).pkt_dts = (*pkt).dts;
                (*frame).duration = (*pkt).duration;
                (*frame).buf[0] = av_buffer_alloc((*pkt).size as usize);
                (*frame).data[0] = (*(*frame).buf[0]).data;
                (*frame).linesize[0] = (*pkt).size;
                memcpy(
                    (*frame).data[0] as *mut libc::c_void,
                    (*pkt).data as *const libc::c_void,
                    (*pkt).size as usize,
                );
                return Ok(vec![(frame, stream)]);
            }

            let mut ret = avcodec_send_packet(ctx.context, pkt);
            if ret < 0 {
                return Err(Error::msg(format!("Failed to decode packet {}", ret)));
            }

            let mut pkgs = Vec::new();
            while ret >= 0 {
                let frame = av_frame_alloc();
                ret = avcodec_receive_frame(ctx.context, frame);
                if ret < 0 {
                    if ret == AVERROR_EOF || ret == AVERROR(libc::EAGAIN) {
                        break;
                    }
                    return Err(Error::msg(format!("Failed to decode {}", ret)));
                }

                (*frame).pict_type = AV_PICTURE_TYPE_NONE; // encoder prints warnings
                pkgs.push((frame, stream));
            }
            Ok(pkgs)
        } else {
            Ok(vec![])
        }
    }
}
