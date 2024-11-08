use anyhow::Error;
use ffmpeg_sys_the_third::{
    av_dict_set, av_frame_alloc, av_frame_copy_props, av_frame_free, av_hwframe_transfer_data,
    av_make_error_string, av_opt_next, av_opt_set, AVDictionary, AVFrame, AVOption,
    AV_OPT_SEARCH_CHILDREN,
};
use std::collections::HashMap;
use std::ptr;

mod decode;
mod demux;
mod encode;
mod filter;
mod resample;
mod scale;
mod stream_info;

#[macro_export]
macro_rules! return_ffmpeg_error {
    ($x:expr) => {
        if $x < 0 {
            anyhow::bail!(crate::get_ffmpeg_error_msg($x))
        }
    };
    ($x:expr,$msg:expr) => {
        if $x < 0 {
            anyhow::bail!(format!("{}: {}", $msg, crate::get_ffmpeg_error_msg($x)))
        }
    };
}

#[macro_export]
macro_rules! cstr {
    ($str:expr) => {
        format!("{}\0", $str).as_ptr() as *const libc::c_char
    };
}

#[macro_export]
macro_rules! rstr {
    ($str:expr) => {
        if !$str.is_null() {
            core::ffi::CStr::from_ptr($str).to_str().unwrap()
        } else {
            ""
        }
    };
}

fn get_ffmpeg_error_msg(ret: libc::c_int) -> String {
    unsafe {
        const BUF_SIZE: usize = 512;
        let mut buf: [libc::c_char; BUF_SIZE] = [0; BUF_SIZE];
        av_make_error_string(buf.as_mut_ptr(), BUF_SIZE, ret);
        rstr!(buf.as_ptr()).to_string()
    }
}

unsafe fn options_to_dict(options: HashMap<String, String>) -> Result<*mut AVDictionary, Error> {
    let mut dict = ptr::null_mut();
    for (key, value) in options {
        let ret = av_dict_set(&mut dict, cstr!(key), cstr!(value), 0);
        return_ffmpeg_error!(ret);
    }
    Ok(dict)
}

/// Format seconds value into human-readable string
pub fn format_time(secs: f32) -> String {
    const MIN: f32 = 60.0;
    const HR: f32 = MIN * 60.0;

    if secs >= HR {
        format!(
            "{:0>2.0}h {:0>2.0}m {:0>2.0}s",
            (secs / HR).floor(),
            ((secs % HR) / MIN).floor(),
            (secs % MIN).floor()
        )
    } else if secs >= MIN {
        format!(
            "{:0>2.0}m {:0>2.0}s",
            (secs / MIN).floor(),
            (secs % MIN).floor()
        )
    } else {
        format!("{:0>2.2}s", secs)
    }
}

fn list_opts(ctx: *mut libc::c_void) -> Result<Vec<String>, Error> {
    let mut opt_ptr: *const AVOption = ptr::null_mut();

    let mut ret = vec![];
    unsafe {
        loop {
            opt_ptr = av_opt_next(ctx, opt_ptr);
            if opt_ptr.is_null() {
                break;
            }

            ret.push(rstr!((*opt_ptr).name).to_string());
        }
    }
    Ok(ret)
}

fn set_opts(ctx: *mut libc::c_void, options: HashMap<String, String>) -> Result<(), Error> {
    unsafe {
        for (key, value) in options {
            let ret = av_opt_set(ctx, cstr!(key), cstr!(value), AV_OPT_SEARCH_CHILDREN);
            return_ffmpeg_error!(ret);
        }
    }
    Ok(())
}

/// Get the frame as CPU frame
pub unsafe fn get_frame_from_hw(mut frame: *mut AVFrame) -> Result<*mut AVFrame, Error> {
    if (*frame).hw_frames_ctx.is_null() {
        Ok(frame)
    } else {
        let new_frame = av_frame_alloc();
        let ret = av_hwframe_transfer_data(new_frame, frame, 0);
        return_ffmpeg_error!(ret);
        av_frame_copy_props(new_frame, frame);
        av_frame_free(&mut frame);
        Ok(new_frame)
    }
}

#[cfg(test)]
pub unsafe fn generate_test_frame() -> *mut AVFrame {
    use ffmpeg_sys_the_third::{av_frame_get_buffer, AVPixelFormat};
    use std::mem::transmute;

    let frame = av_frame_alloc();
    (*frame).width = 512;
    (*frame).height = 512;
    (*frame).format = transmute(AVPixelFormat::AV_PIX_FMT_RGB24);
    av_frame_get_buffer(frame, 0);

    let mut lx = 0;
    for line in 0..(*frame).height {
        let c = lx % 3;
        for y in 0..(*frame).width as usize {
            let ptr = (*frame).data[0];
            let offset = (line * (*frame).linesize[0]) as usize + (y * 3);
            match c {
                0 => *ptr.add(offset) = 0xff,
                1 => *ptr.add(offset + 1) = 0xff,
                2 => *ptr.add(offset + 2) = 0xff,
                _ => {}
            }
        }
        lx += 1;
    }

    frame
}

pub use decode::*;
pub use demux::*;
pub use ffmpeg_sys_the_third;
pub use filter::*;
pub use resample::*;
pub use scale::*;
pub use stream_info::*;
