use crate::{
    AvFrameRef, AvPacketRef, bail_ffmpeg, cstr, free_cstr, get_ffmpeg_error_msg, options_to_dict,
};
use anyhow::{Error, Result, bail};
use ffmpeg_sys_the_third::AVPictureType::AV_PICTURE_TYPE_NONE;
use ffmpeg_sys_the_third::{
    AVChannelLayout, AVCodec, AVCodecContext, AVCodecID, AVERROR, AVERROR_EOF, AVFrame,
    AVPixelFormat, AVRational, AVSampleFormat, av_channel_layout_default, av_d2q, av_inv_q,
    av_packet_alloc, av_packet_free, avcodec_alloc_context3, avcodec_find_encoder,
    avcodec_find_encoder_by_name, avcodec_free_context, avcodec_open2, avcodec_receive_packet,
    avcodec_send_frame,
};
#[cfg(feature = "avcodec_version_greater_than_61_13")]
use ffmpeg_sys_the_third::{AVCodecConfig, avcodec_get_supported_config};

use libc::EAGAIN;
use std::collections::HashMap;
use std::io::Write;
use std::{ptr, slice};

pub struct Encoder {
    ctx: *mut AVCodecContext,
    codec: *const AVCodec,
    dst_stream_index: Option<i32>,
}

unsafe impl Send for Encoder {}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                avcodec_free_context(&mut self.ctx);
            }
        }
    }
}

impl Encoder {
    /// Create a new encoder by codec id
    pub fn new(codec: AVCodecID) -> Result<Self> {
        unsafe {
            let codec = avcodec_find_encoder(codec);
            if codec.is_null() {
                bail!("codec not found");
            }
            Self::new_with_codec(codec)
        }
    }

    /// Create a new encoder by name
    pub fn new_with_name(name: &str) -> Result<Self> {
        let c_name = cstr!(name);
        let codec = unsafe { avcodec_find_encoder_by_name(c_name) };
        free_cstr!(c_name);
        if codec.is_null() {
            bail!("codec not found");
        }
        Self::new_with_codec(codec)
    }

    /// Create a new encoder with a specific codec instance
    pub fn new_with_codec(codec: *const AVCodec) -> Result<Self> {
        unsafe {
            let ctx = avcodec_alloc_context3(codec);
            if ctx.is_null() {
                bail!("context allocation failed");
            }
            // set some defaults
            (*ctx).time_base = AVRational { num: 1, den: 1 };
            Ok(Self {
                ctx,
                codec,
                dst_stream_index: None,
            })
        }
    }

    /// Get the codec
    pub fn codec(&self) -> *const AVCodec {
        self.codec
    }

    /// Get the codec context
    pub fn codec_context(&self) -> *const AVCodecContext {
        self.ctx
    }

    #[cfg(feature = "avcodec_version_greater_than_61_13")]
    /// List supported configs (see [avcodec_get_supported_config])
    pub unsafe fn list_configs<'a, T>(&mut self, cfg: AVCodecConfig) -> Result<&'a [T], Error> {
        unsafe {
            let mut dst = ptr::null_mut();
            let mut num_dst = 0;
            let ret = avcodec_get_supported_config(
                self.ctx,
                self.codec,
                cfg,
                0,
                ptr::addr_of_mut!(dst) as _,
                &mut num_dst,
            );
            bail_ffmpeg!(ret);
            Ok(slice::from_raw_parts(dst, num_dst as usize))
        }
    }

    /// Store the destination stream index along with the encoder
    /// AVPacket's created by this encoder will have stream_index assigned to this value
    pub unsafe fn with_stream_index(mut self, index: i32) -> Self {
        self.dst_stream_index = Some(index);
        self
    }

    /// Set the encoder bitrate
    pub unsafe fn with_bitrate(self, bitrate: i64) -> Self {
        unsafe {
            (*self.ctx).bit_rate = bitrate;
            self
        }
    }

    /// Set the encoder sample rate (audio)
    pub unsafe fn with_sample_rate(self, rate: i32) -> Result<Self> {
        unsafe {
            if (*self.ctx).time_base.num != 1 || (*self.ctx).time_base.den != 1 {
                bail!("Cannot assign sample_rate for a video encoder")
            }
            (*self.ctx).sample_rate = rate;
            (*self.ctx).time_base = AVRational { num: 1, den: rate };
            Ok(self)
        }
    }

    /// Set the encoder width in pixels
    pub unsafe fn with_width(self, width: i32) -> Self {
        unsafe {
            (*self.ctx).width = width;
            self
        }
    }

    /// Set the encoder height in pixels
    pub unsafe fn with_height(self, height: i32) -> Self {
        unsafe {
            (*self.ctx).height = height;
            self
        }
    }

    /// Set the encoder level (see AV_LEVEL_*)
    pub unsafe fn with_level(self, level: i32) -> Self {
        unsafe {
            (*self.ctx).level = level;
            self
        }
    }

    /// Set the encoder profile (see AV_PROFILE_*)
    pub unsafe fn with_profile(self, profile: i32) -> Self {
        unsafe {
            (*self.ctx).profile = profile;
            self
        }
    }

    /// Set the encoder framerate
    pub unsafe fn with_framerate(self, fps: f32) -> Result<Self> {
        unsafe {
            if (*self.ctx).time_base.num != 1 || (*self.ctx).time_base.den != 1 {
                bail!("Cannot assign framerate for an audio encoder")
            }
            let q = av_d2q(fps as f64, 90_000);
            (*self.ctx).framerate = q;
            (*self.ctx).time_base = av_inv_q(q);
            Ok(self)
        }
    }

    /// Set the encoder pixel format
    pub unsafe fn with_pix_fmt(self, fmt: AVPixelFormat) -> Self {
        unsafe {
            (*self.ctx).pix_fmt = fmt;
            self
        }
    }

    /// Set the encoder sample format (audio)
    pub unsafe fn with_sample_format(self, fmt: AVSampleFormat) -> Self {
        unsafe {
            (*self.ctx).sample_fmt = fmt;
            self
        }
    }

    /// Set the encoder channel layout (audio)
    pub unsafe fn with_channel_layout(self, layout: AVChannelLayout) -> Self {
        unsafe {
            (*self.ctx).ch_layout = layout;
            self
        }
    }

    /// Set the encoder channel layout using number of channels (audio)
    pub unsafe fn with_default_channel_layout(self, channels: i32) -> Self {
        unsafe {
            let mut layout = AVChannelLayout::empty();
            av_channel_layout_default(&mut layout, channels);
            (*self.ctx).ch_layout = layout;
            self
        }
    }

    /// Apply options to context
    pub unsafe fn with_options<F>(self, fx: F) -> Self
    where
        F: FnOnce(*mut AVCodecContext),
    {
        fx(self.ctx);
        self
    }

    /// Open the encoder so that you can start encoding frames (see [avcodec_open2])
    pub unsafe fn open(self, options: Option<HashMap<String, String>>) -> Result<Self> {
        unsafe {
            assert!(!self.ctx.is_null());

            let mut options = if let Some(options) = options {
                options_to_dict(options)?
            } else {
                ptr::null_mut()
            };
            let ret = avcodec_open2(self.ctx, self.codec, &mut options);
            bail_ffmpeg!(ret);
            Ok(self)
        }
    }

    /// Encode a frame, returning a number of [AvPacketRef]
    /// MAKE SURE TIMESTAMP ARE SET CORRECTLY
    pub fn encode_frame(&mut self, frame: Option<&AvFrameRef>) -> Result<Vec<AvPacketRef>> {
        match frame {
            Some(f) => {
                // always reset pict_type, this can be set by the decoder,
                // but it confuses the encoder
                let mut f_clone = f.clone();
                f_clone.pict_type = AV_PICTURE_TYPE_NONE;
                unsafe { self.encode_frame_internal(f_clone.ptr()) }
            }
            None => unsafe { self.encode_frame_internal(ptr::null_mut()) },
        }
    }

    unsafe fn encode_frame_internal(&mut self, frame: *mut AVFrame) -> Result<Vec<AvPacketRef>> {
        unsafe {
            let mut packets = Vec::new();
            let mut ret = avcodec_send_frame(self.ctx, frame);
            if ret < 0 && ret != AVERROR(EAGAIN) {
                bail!(get_ffmpeg_error_msg(ret));
            }

            while ret >= 0 || ret == AVERROR(EAGAIN) {
                let pkt = av_packet_alloc();
                ret = avcodec_receive_packet(self.ctx, pkt);
                if ret != 0 {
                    av_packet_free(&mut (pkt as *mut _));
                    if ret == AVERROR(EAGAIN) || ret == AVERROR_EOF {
                        break;
                    }
                    bail!(get_ffmpeg_error_msg(ret));
                }
                (*pkt).time_base = (*self.ctx).time_base;
                if (*pkt).duration == 0 {
                    (*pkt).duration = 1; // Set duration to 1 for video packets (CFR) if not already set
                }
                if let Some(idx) = self.dst_stream_index {
                    (*pkt).stream_index = idx;
                }
                packets.push(AvPacketRef::new(pkt));
            }
            Ok(packets)
        }
    }

    /// Encode a single frame and write it to disk
    pub fn save_picture(mut self, frame: &AvFrameRef, dst: &str) -> Result<()> {
        let mut fout = std::fs::File::create(dst)?;

        for pkt in self.encode_frame(Some(frame))? {
            let pkt_slice = unsafe { slice::from_raw_parts(pkt.data, pkt.size as usize) };
            fout.write_all(pkt_slice)?;
        }
        for pkt in self.encode_frame(None)? {
            let pkt_slice = unsafe { slice::from_raw_parts(pkt.data, pkt.size as usize) };
            fout.write_all(pkt_slice)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_test_frame;

    #[test]
    fn test_encode_png() -> Result<(), Error> {
        let frame = unsafe { generate_test_frame() };
        let mut encoder = Encoder::new(AVCodecID::AV_CODEC_ID_PNG)?;
        encoder = unsafe { encoder.with_width(frame.width).with_height(frame.height) };

        #[cfg(feature = "avcodec_version_greater_than_61_13")]
        let pix_fmts: &[AVPixelFormat] =
            unsafe { encoder.list_configs(AVCodecConfig::AV_CODEC_CONFIG_PIX_FORMAT)? };
        #[cfg(not(feature = "avcodec_version_greater_than_61_13"))]
        let pix_fmts = [AV_PIX_FMT_YUV420P];

        encoder = unsafe { encoder.with_pix_fmt(pix_fmts[0]).open(None)? };

        std::fs::create_dir_all("test_output")?;
        encoder.save_picture(&frame, "test_output/test.png")?;
        Ok(())
    }
}
