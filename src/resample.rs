use crate::get_ffmpeg_error_msg;
use crate::return_ffmpeg_error;
use anyhow::Error;
use ffmpeg_sys_the_third::{
    av_channel_layout_default, av_frame_alloc, av_frame_copy_props, av_frame_free,
    swr_alloc_set_opts2, swr_convert_frame, swr_init, AVChannelLayout, AVFrame, AVSampleFormat,
    SwrContext,
};
use libc::malloc;
use std::mem::transmute;
use std::ptr;

pub struct Resample {
    format: AVSampleFormat,
    sample_rate: u32,
    channels: usize,
    ctx: *mut SwrContext,
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
        let layout = malloc(size_of::<AVChannelLayout>()) as *mut AVChannelLayout;
        av_channel_layout_default(layout, self.channels as libc::c_int);

        let ret = swr_alloc_set_opts2(
            &mut self.ctx,
            layout,
            self.format,
            self.sample_rate as libc::c_int,
            &(*frame).ch_layout,
            transmute((*frame).format),
            (*frame).sample_rate,
            0,
            ptr::null_mut(),
        );
        return_ffmpeg_error!(ret);

        let ret = swr_init(self.ctx);
        return_ffmpeg_error!(ret);

        Ok(())
    }

    pub unsafe fn process_frame(&mut self, frame: *mut AVFrame) -> Result<*mut AVFrame, Error> {
        if !(*frame).hw_frames_ctx.is_null() {
            anyhow::bail!("Hardware frames are not supported in this software re-sampler");
        }
        self.setup_swr(frame)?;

        let mut out_frame = av_frame_alloc();
        av_frame_copy_props(out_frame, frame);
        (*out_frame).sample_rate = self.sample_rate as libc::c_int;
        (*out_frame).format = transmute(self.format);
        (*out_frame).time_base = (*frame).time_base;

        av_channel_layout_default(&mut (*out_frame).ch_layout, self.channels as libc::c_int);

        let ret = swr_convert_frame(self.ctx, out_frame, frame);
        if ret < 0 {
            av_frame_free(&mut out_frame);
            return Err(Error::msg(get_ffmpeg_error_msg(ret)));
        }

        Ok(out_frame)
    }
}
