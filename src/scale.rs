use std::mem::transmute;
use std::ptr;

use crate::return_ffmpeg_error;
use anyhow::{bail, Error};
use ffmpeg_sys_the_third::{
    av_frame_alloc, av_frame_copy_props, sws_freeContext, sws_getContext, sws_init_context,
    sws_scale_frame, AVFrame, AVPixelFormat, SwsContext, SWS_BILINEAR,
};

pub struct Scaler {
    width: u16,
    height: u16,
    format: AVPixelFormat,
    ctx: *mut SwsContext,
}

unsafe impl Send for Scaler {}

unsafe impl Sync for Scaler {}

impl Drop for Scaler {
    fn drop(&mut self) {
        unsafe {
            sws_freeContext(self.ctx);
            self.ctx = ptr::null_mut();
        }
    }
}

impl Scaler {
    pub fn new(width: u16, height: u16, format: AVPixelFormat) -> Self {
        Self {
            width,
            height,
            format,
            ctx: ptr::null_mut(),
        }
    }

    unsafe fn setup_scaler(
        &mut self,
        frame: *const AVFrame,
        width: u16,
        height: u16,
        format: AVPixelFormat,
    ) -> Result<(), Error> {
        if !self.ctx.is_null()
            && self.width == width
            && self.height == height
            && self.format == format
        {
            return Ok(());
        }

        // clear previous context, before re-creating
        if !self.ctx.is_null() {
            sws_freeContext(self.ctx);
            self.ctx = ptr::null_mut();
        }

        self.ctx = sws_getContext(
            (*frame).width,
            (*frame).height,
            transmute((*frame).format),
            self.width as libc::c_int,
            self.height as libc::c_int,
            transmute(self.format),
            SWS_BILINEAR,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if self.ctx.is_null() {
            bail!("Failed to create scalar context");
        }

        self.width = width;
        self.height = height;
        self.format = format;
        Ok(())
    }

    pub unsafe fn process_frame(
        &mut self,
        frame: *mut AVFrame,
        width: u16,
        height: u16,
        format: AVPixelFormat,
    ) -> Result<*mut AVFrame, Error> {
        if !(*frame).hw_frames_ctx.is_null() {
            bail!("Hardware frames are not supported in this software scalar");
        }

        self.setup_scaler(frame, width, height, format)?;

        let dst_frame = av_frame_alloc();
        let ret = av_frame_copy_props(dst_frame, frame);
        return_ffmpeg_error!(ret);

        let ret = sws_scale_frame(self.ctx, dst_frame, frame);
        return_ffmpeg_error!(ret);

        Ok(dst_frame)
    }
}
