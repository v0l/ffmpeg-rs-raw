use crate::{bail_ffmpeg, cstr, set_opts, Encoder};
use anyhow::{bail, Result};
use ffmpeg_sys_the_third::{
    av_dump_format, av_interleaved_write_frame, av_packet_rescale_ts, av_write_trailer,
    avcodec_parameters_copy, avcodec_parameters_from_context, avformat_alloc_output_context2,
    avformat_free_context, avformat_new_stream, avformat_write_header, avio_flush, avio_open,
    AVFormatContext, AVPacket, AVStream, AVFMT_GLOBALHEADER, AVFMT_NOFILE, AVIO_FLAG_WRITE,
    AV_CODEC_FLAG_GLOBAL_HEADER,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::ptr;

pub struct Muxer {
    ctx: *mut AVFormatContext,
}

impl Drop for Muxer {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                avio_flush((*self.ctx).pb);
                avformat_free_context(self.ctx);
            }
        }
    }
}

impl Default for Muxer {
    fn default() -> Self {
        Self::new()
    }
}

impl Muxer {
    pub fn new() -> Self {
        Self {
            ctx: ptr::null_mut(),
        }
    }

    /// Open the muxer with a destination path
    pub unsafe fn with_output(
        mut self,
        dst: &PathBuf,
        format: Option<&str>,
        options: Option<HashMap<String, String>>,
    ) -> Result<Self> {
        if !self.ctx.is_null() {
            bail!("context already open");
        }

        let ret = avformat_alloc_output_context2(
            &mut self.ctx,
            ptr::null_mut(),
            if let Some(ref format) = format {
                cstr!(format)
            } else {
                ptr::null()
            },
            cstr!(dst.to_str().unwrap()),
        );
        bail_ffmpeg!(ret);

        // Setup global header flag
        if (*(*self.ctx).oformat).flags & AVFMT_GLOBALHEADER != 0 {
            (*self.ctx).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as libc::c_int;
        }

        // Set options on ctx
        if let Some(opts) = options {
            set_opts((*self.ctx).priv_data, opts)?;
        }

        Ok(self)
    }

    /// Add a stream to the output using an existing encoder
    pub unsafe fn with_stream_encoder(mut self, encoder: &Encoder) -> Result<Self> {
        self.add_stream_encoder(encoder)?;
        Ok(self)
    }

    /// Add a stream to the output using an existing encoder
    pub unsafe fn add_stream_encoder(&mut self, encoder: &Encoder) -> Result<*mut AVStream> {
        let stream = avformat_new_stream(self.ctx, encoder.codec());
        if stream.is_null() {
            bail!("unable to allocate stream");
        }
        let ret = avcodec_parameters_from_context((*stream).codecpar, encoder.codec_context());
        bail_ffmpeg!(ret);

        // setup other stream params
        let encoder_ctx = encoder.codec_context();
        (*stream).time_base = (*encoder_ctx).time_base;
        (*stream).avg_frame_rate = (*encoder_ctx).framerate;
        (*stream).r_frame_rate = (*encoder_ctx).framerate;

        Ok(stream)
    }

    /// Add a stream to the output using an existing input stream (copy)
    pub unsafe fn add_copy_stream(&mut self, in_stream: *mut AVStream) -> Result<*mut AVStream> {
        let stream = avformat_new_stream(self.ctx, ptr::null_mut());
        if stream.is_null() {
            bail!("unable to allocate stream");
        }

        // copy params from input
        let ret = avcodec_parameters_copy((*stream).codecpar, (*in_stream).codecpar);
        bail_ffmpeg!(ret);

        Ok(stream)
    }

    /// Open the output to start sending packets
    pub unsafe fn open(&mut self) -> Result<()> {
        if (*(*self.ctx).oformat).flags & AVFMT_NOFILE == 0 {
            let ret = avio_open(&mut (*self.ctx).pb, (*self.ctx).url, AVIO_FLAG_WRITE);
            bail_ffmpeg!(ret);
        }

        let ret = avformat_write_header(self.ctx, ptr::null_mut());
        bail_ffmpeg!(ret);

        av_dump_format(self.ctx, 0, (*self.ctx).url, 1);

        Ok(())
    }

    /// Write a packet to the output
    pub unsafe fn write_packet(&mut self, pkt: *mut AVPacket) -> Result<()> {
        let stream = *(*self.ctx).streams.add((*pkt).stream_index as usize);
        av_packet_rescale_ts(pkt, (*pkt).time_base, (*stream).time_base);
        (*pkt).time_base = (*stream).time_base;

        let ret = av_interleaved_write_frame(self.ctx, pkt);
        bail_ffmpeg!(ret);
        Ok(())
    }

    /// Close the output and write the trailer
    pub unsafe fn close(self) -> Result<()> {
        let ret = av_write_trailer(self.ctx);
        bail_ffmpeg!(ret);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_test_frame, Scaler};
    use ffmpeg_sys_the_third::AVCodecID::AV_CODEC_ID_H264;
    use ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
    use ffmpeg_sys_the_third::AV_PROFILE_H264_MAIN;

    #[test]
    fn encode_mkv() -> Result<()> {
        std::fs::create_dir_all("test_output")?;
        unsafe {
            let path = PathBuf::from("test_output/test.mp4");
            let frame = generate_test_frame();

            // convert frame to YUV
            let mut scaler = Scaler::new();
            let frame = scaler.process_frame(
                frame,
                (*frame).width as u16,
                (*frame).height as u16,
                AV_PIX_FMT_YUV420P,
            )?;

            let mut encoder = Encoder::new(AV_CODEC_ID_H264)?
                .with_width((*frame).width)
                .with_height((*frame).height)
                .with_pix_fmt(AV_PIX_FMT_YUV420P)
                .with_bitrate(1_000_000)
                .with_framerate(30.0)
                .with_profile(AV_PROFILE_H264_MAIN)
                .with_level(50)
                .open(None)?;

            let mut muxer = Muxer::new()
                .with_output(&path, None, None)?
                .with_stream_encoder(&encoder)?;
            muxer.open()?;
            let mut pts = 0;
            for z in 0..100 {
                (*frame).pts = pts;
                for pkt in encoder.encode_frame(frame)? {
                    muxer.write_packet(pkt)?;
                }
                pts += 1;
            }
            // flush
            for f_pk in encoder.encode_frame(ptr::null_mut())? {
                muxer.write_packet(f_pk)?;
            }
            muxer.close()?;
        }
        Ok(())
    }
}
