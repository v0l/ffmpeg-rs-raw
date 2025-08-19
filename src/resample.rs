use crate::bail_ffmpeg;
use anyhow::Error;
use ffmpeg_sys_the_third::{
    AVChannelLayout, AVFrame, AVSampleFormat, SwrContext, av_channel_layout_default,
    av_frame_alloc, av_frame_copy_props, av_frame_free, swr_alloc_set_opts2, swr_convert_frame,
    swr_free, swr_init,
};
use std::mem::transmute;
use std::ptr;

pub struct Resample {
    format: AVSampleFormat,
    sample_rate: u32,
    channels: usize,
    ctx: *mut SwrContext,
}

impl Drop for Resample {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                swr_free(&mut self.ctx);
            }
        }
    }
}

impl Resample {
    pub fn new(format: AVSampleFormat, rate: u32, channels: usize) -> Self {
        Self {
            format,
            channels,
            sample_rate: rate,
            ctx: ptr::null_mut(),
        }
    }

    unsafe fn setup_swr(&mut self, frame: *mut AVFrame) -> Result<(), Error> {
        if !self.ctx.is_null() {
            return Ok(());
        }
        let mut layout = AVChannelLayout::empty();
        av_channel_layout_default(&mut layout, self.channels as libc::c_int);

        let ret = swr_alloc_set_opts2(
            &mut self.ctx,
            ptr::addr_of_mut!(layout),
            self.format,
            self.sample_rate as libc::c_int,
            ptr::addr_of_mut!((*frame).ch_layout),
            transmute((*frame).format),
            (*frame).sample_rate,
            0,
            ptr::null_mut(),
        );
        bail_ffmpeg!(ret);

        let ret = swr_init(self.ctx);
        bail_ffmpeg!(ret);

        Ok(())
    }

    /// Resample an audio frame
    pub unsafe fn process_frame(&mut self, frame: *mut AVFrame) -> Result<*mut AVFrame, Error> {
        if !(*frame).hw_frames_ctx.is_null() {
            anyhow::bail!("Hardware frames are not supported in this software re-sampler");
        }
        self.setup_swr(frame)?;

        let mut out_frame = av_frame_alloc();
        av_frame_copy_props(out_frame, frame);
        (*out_frame).sample_rate = self.sample_rate as libc::c_int;
        (*out_frame).format = transmute(self.format);
        av_channel_layout_default(&mut (*out_frame).ch_layout, self.channels as libc::c_int);

        let ret = swr_convert_frame(self.ctx, out_frame, frame);
        bail_ffmpeg!(ret, {
            av_frame_free(&mut out_frame);
        });

        Ok(out_frame)
    }
}
