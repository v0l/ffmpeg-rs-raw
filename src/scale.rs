use std::mem::transmute;
use std::ptr;

use crate::{bail_ffmpeg, rstr};
use anyhow::{bail, Error};
use ffmpeg_sys_the_third::{
    av_frame_alloc, av_frame_copy_props, av_get_pix_fmt_name, sws_freeContext, sws_getContext,
    sws_scale_frame, AVFrame, AVPixelFormat, SwsContext, SWS_BILINEAR,
};
use log::trace;

pub struct Scaler {
    width: u16,
    height: u16,
    format: AVPixelFormat,
    ctx: *mut SwsContext,
}

impl Drop for Scaler {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                sws_freeContext(self.ctx);
            }
        }
    }
}

impl Default for Scaler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scaler {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            format: AVPixelFormat::AV_PIX_FMT_YUV420P,
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
            width as libc::c_int,
            height as libc::c_int,
            transmute(format),
            SWS_BILINEAR,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if self.ctx.is_null() {
            bail!("Failed to create scalar context");
        }

        trace!(
            "scale setup: {}x{}@{} => {}x{}@{}",
            (*frame).width,
            (*frame).height,
            rstr!(av_get_pix_fmt_name(transmute((*frame).format))),
            width,
            height,
            rstr!(av_get_pix_fmt_name(format))
        );

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
        bail_ffmpeg!(ret);

        let ret = sws_scale_frame(self.ctx, dst_frame, frame);
        bail_ffmpeg!(ret);

        Ok(dst_frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_test_frame;
    use ffmpeg_sys_the_third::{av_frame_free, AVPixelFormat};

    #[test]
    fn scale_rgb24_yuv420() {
        unsafe {
            let mut frame = generate_test_frame();
            let mut scaler = Scaler::new();

            // downscale
            let mut out_frame = scaler
                .process_frame(frame, 128, 128, AVPixelFormat::AV_PIX_FMT_YUV420P)
                .expect("Failed to process frame");
            assert_eq!((*out_frame).width, 128);
            assert_eq!((*out_frame).height, 128);
            assert_eq!(
                (*out_frame).format,
                transmute(AVPixelFormat::AV_PIX_FMT_YUV420P)
            );
            av_frame_free(&mut out_frame);

            // upscale
            let mut out_frame = scaler
                .process_frame(frame, 1024, 1024, AVPixelFormat::AV_PIX_FMT_YUV420P)
                .expect("Failed to process frame");
            assert_eq!((*out_frame).width, 1024);
            assert_eq!((*out_frame).height, 1024);
            assert_eq!(
                (*out_frame).format,
                transmute(AVPixelFormat::AV_PIX_FMT_YUV420P)
            );
            av_frame_free(&mut out_frame);

            av_frame_free(&mut frame);
        }
    }
}
