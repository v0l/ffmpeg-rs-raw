use crate::{bail_ffmpeg, options_to_dict, rstr, StreamInfo};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::ptr;

use anyhow::Error;
use ffmpeg_sys_the_third::{
    av_buffer_ref, av_frame_alloc, av_frame_free, av_hwdevice_ctx_create,
    av_hwdevice_get_type_name, av_hwdevice_iterate_types, avcodec_alloc_context3,
    avcodec_find_decoder, avcodec_free_context, avcodec_get_hw_config, avcodec_get_name,
    avcodec_open2, avcodec_parameters_to_context, avcodec_receive_frame, avcodec_send_packet,
    AVCodec, AVCodecContext, AVCodecHWConfig, AVFrame, AVHWDeviceType, AVPacket, AVStream, AVERROR,
    AVERROR_EOF, AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX,
};
use log::trace;

pub struct DecoderCodecContext {
    pub context: *mut AVCodecContext,
    pub codec: *const AVCodec,
    pub stream: *mut AVStream,
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

    /// Get the codec name
    pub fn codec_name(&self) -> String {
        let codec_name = unsafe { rstr!((*self.codec).name) };
        if self.hw_config.is_null() {
            codec_name.to_string()
        } else {
            let hw = unsafe { rstr!(av_hwdevice_get_type_name((*self.hw_config).device_type)) };
            format!("{}_{}", codec_name, hw)
        }
    }
}

impl Drop for DecoderCodecContext {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                avcodec_free_context(&mut self.context);
            }
        }
    }
}

impl Display for DecoderCodecContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stream={}, codec={}",
            unsafe { (*self.stream).index },
            self.codec_name()
        )
    }
}

pub struct Decoder {
    /// Decoder instances by stream index
    codecs: HashMap<i32, DecoderCodecContext>,
    /// List of [AVHWDeviceType] which are enabled
    hw_decoder_types: Option<HashSet<AVHWDeviceType>>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
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
        channel: &StreamInfo,
        options: Option<HashMap<String, String>>,
    ) -> Result<&mut DecoderCodecContext, Error> {
        unsafe { self.setup_decoder_for_stream(channel.stream, options) }
    }

    /// Get the codec context of a stream by stream index
    pub fn get_decoder(&self, stream: i32) -> Option<&DecoderCodecContext> {
        self.codecs.get(&stream)
    }

    /// List supported hardware decoding for a given codec instance
    pub unsafe fn list_supported_hw_accel(
        &self,
        codec: *const AVCodec,
    ) -> impl Iterator<Item = AVHWDeviceType> {
        let mut hw_config = ptr::null();
        let mut i = 0;
        let mut ret = Vec::new();
        loop {
            hw_config = avcodec_get_hw_config(codec, i);
            i += 1;
            if hw_config.is_null() {
                break;
            }
            let hw_flag = AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as libc::c_int;
            if (*hw_config).methods & hw_flag == hw_flag {
                ret.push((*hw_config).device_type);
            }
        }
        ret.into_iter()
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

        if let Entry::Vacant(e) = self.codecs.entry((*stream).index) {
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
            bail_ffmpeg!(ret, "Failed to copy codec parameters to context");

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
                        trace!("skipping hwaccel={}_{}", codec_name, hw_name);
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
                        bail_ffmpeg!(ret, "Failed to create HW ctx");
                        (*context).hw_device_ctx = av_buffer_ref(hw_buf_ref);
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
            bail_ffmpeg!(ret, "Failed to open codec");

            let ctx = DecoderCodecContext {
                context,
                codec,
                stream,
                hw_config,
            };
            trace!("setup decoder={}", ctx);
            Ok(e.insert(ctx))
        } else {
            anyhow::bail!("Decoder already setup");
        }
    }

    /// Flush all decoders
    pub unsafe fn flush(&mut self) -> Result<Vec<(*mut AVFrame, *mut AVStream)>, Error> {
        let mut pkgs = Vec::new();
        for ctx in self.codecs.values_mut() {
            pkgs.extend(Self::decode_pkt_internal(ctx, ptr::null_mut())?);
        }
        Ok(pkgs)
    }

    pub unsafe fn decode_pkt_internal(
        ctx: &DecoderCodecContext,
        pkt: *mut AVPacket,
    ) -> Result<Vec<(*mut AVFrame, *mut AVStream)>, Error> {
        let mut ret = avcodec_send_packet(ctx.context, pkt);
        bail_ffmpeg!(ret, "Failed to decode packet");

        let mut pkgs = Vec::new();
        while ret >= 0 {
            let mut frame = av_frame_alloc();
            ret = avcodec_receive_frame(ctx.context, frame);
            if ret < 0 {
                av_frame_free(&mut frame);
                if ret == AVERROR_EOF || ret == AVERROR(libc::EAGAIN) {
                    break;
                }
                return Err(Error::msg(format!("Failed to decode {}", ret)));
            }
            pkgs.push((frame, ctx.stream));
        }
        Ok(pkgs)
    }

    pub unsafe fn decode_pkt(
        &mut self,
        pkt: *mut AVPacket,
    ) -> Result<Vec<(*mut AVFrame, *mut AVStream)>, Error> {
        if pkt.is_null() {
            return self.flush();
        }
        if let Some(ctx) = self.codecs.get_mut(&(*pkt).stream_index) {
            Self::decode_pkt_internal(ctx, pkt)
        } else {
            Ok(vec![])
        }
    }
}
