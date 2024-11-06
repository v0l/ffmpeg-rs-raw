use crate::{options_to_dict, return_ffmpeg_error, rstr, StreamInfoChannel};
use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::fmt::{Display, Formatter};
use std::ptr;

use anyhow::Error;
use ffmpeg_sys_the_third::AVPictureType::AV_PICTURE_TYPE_NONE;
use ffmpeg_sys_the_third::{
    av_buffer_ref, av_frame_alloc, av_hwdevice_ctx_create, av_hwdevice_get_type_name,
    av_hwdevice_iterate_types, avcodec_alloc_context3, avcodec_find_decoder, avcodec_free_context,
    avcodec_get_hw_config, avcodec_get_name, avcodec_open2, avcodec_parameters_to_context,
    avcodec_receive_frame, avcodec_send_packet, AVCodec, AVCodecContext, AVCodecHWConfig, AVFrame,
    AVHWDeviceType, AVPacket, AVStream, AVERROR, AVERROR_EOF,
    AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX,
};
use log::debug;

pub struct DecoderCodecContext {
    pub context: *mut AVCodecContext,
    pub codec: *const AVCodec,
    pub hw_config: *const AVCodecHWConfig,
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

impl Display for DecoderCodecContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        unsafe {
            let codec_name = rstr!(avcodec_get_name((*self.codec).id));
            write!(
                f,
                "DecoderCodecContext: codec={}, hw={}",
                codec_name,
                if self.hw_config.is_null() {
                    "no"
                } else {
                    rstr!(av_hwdevice_get_type_name((*self.hw_config).device_type))
                }
            )
        }
    }
}

unsafe impl Send for DecoderCodecContext {}
unsafe impl Sync for DecoderCodecContext {}

pub struct Decoder {
    codecs: HashMap<i32, DecoderCodecContext>,
    /// List of [AVHWDeviceType] which are enabled
    hw_decoder_types: Option<HashSet<AVHWDeviceType>>,
}

impl Display for Decoder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for (idx, codec) in &self.codecs {
            writeln!(f, "{}: {}", idx, codec)?;
        }
        Ok(())
    }
}
impl Decoder {
    pub fn new() -> Self {
        Self {
            codecs: HashMap::new(),
            hw_decoder_types: None,
        }
    }

    /// Enable hardware decoding with [hw_type]
    pub fn enable_hw_decoder(&mut self, hw_type: AVHWDeviceType) {
        if let Some(ref mut t) = self.hw_decoder_types {
            t.insert(hw_type);
        } else {
            let mut hwt = HashSet::new();
            hwt.insert(hw_type);
            self.hw_decoder_types = Some(hwt);
        }
    }

    /// Enable hardware decoding
    pub fn enable_hw_decoder_any(&mut self) {
        let mut res = HashSet::new();
        let mut hwt = AVHWDeviceType::AV_HWDEVICE_TYPE_NONE;
        unsafe {
            loop {
                hwt = av_hwdevice_iterate_types(hwt);
                if hwt == AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                    break;
                }
                res.insert(hwt);
            }
        }
        self.hw_decoder_types = Some(res);
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
                    rstr!(avcodec_get_name((*codec_par).codec_id))
                )
            }
            let context = avcodec_alloc_context3(codec);
            if context.is_null() {
                anyhow::bail!("Failed to alloc context")
            }

            let mut ret = avcodec_parameters_to_context(context, (*stream).codecpar);
            return_ffmpeg_error!(ret, "Failed to copy codec parameters to context");

            let codec_name = rstr!(avcodec_get_name((*codec).id));
            // try use HW decoder
            let mut hw_config = ptr::null();
            if let Some(ref hw_types) = self.hw_decoder_types {
                let mut hw_buf_ref = ptr::null_mut();
                let mut i = 0;
                loop {
                    hw_config = avcodec_get_hw_config(codec, i);
                    i += 1;
                    if hw_config.is_null() {
                        break;
                    }
                    let hw_name = rstr!(av_hwdevice_get_type_name((*hw_config).device_type));
                    if !hw_types.contains(&(*hw_config).device_type) {
                        debug!("skipping hwaccel={}_{}", codec_name, hw_name);
                        continue;
                    }
                    let hw_flag = AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as libc::c_int;
                    if (*hw_config).methods & hw_flag == hw_flag {
                        ret = av_hwdevice_ctx_create(
                            &mut hw_buf_ref,
                            (*hw_config).device_type,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            0,
                        );
                        return_ffmpeg_error!(ret, "Failed to create HW ctx");
                        (*context).hw_device_ctx = av_buffer_ref(hw_buf_ref);
                        debug!("using hwaccel={}_{}", codec_name, hw_name);
                        break;
                    }
                }
            }
            let mut dict = if let Some(options) = options {
                options_to_dict(options)?
            } else {
                ptr::null_mut()
            };

            ret = avcodec_open2(context, codec, &mut dict);
            return_ffmpeg_error!(ret, "Failed to open codec");

            debug!("opened decoder={}", codec_name);
            Ok(e.insert(DecoderCodecContext {
                context,
                codec,
                hw_config,
            }))
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
