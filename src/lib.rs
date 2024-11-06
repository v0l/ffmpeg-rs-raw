use anyhow::Error;
use ffmpeg_sys_the_third::{av_dict_set, av_frame_alloc, av_frame_copy_props, av_frame_free, av_hwframe_transfer_data, av_make_error_string, av_opt_next, av_opt_set, AVDictionary, AVFrame, AVOption, AV_OPT_SEARCH_CHILDREN};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr;

mod decode;
mod demux;
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

fn get_ffmpeg_error_msg(ret: libc::c_int) -> String {
    unsafe {
        const BUF_SIZE: usize = 512;
        let mut buf: [libc::c_char; BUF_SIZE] = [0; BUF_SIZE];
        av_make_error_string(buf.as_mut_ptr(), BUF_SIZE, ret);
        String::from(CStr::from_ptr(buf.as_ptr()).to_str().unwrap())
    }
}

unsafe fn options_to_dict(options: HashMap<String, String>) -> Result<*mut AVDictionary, Error> {
    let mut dict = ptr::null_mut();
    for (key, value) in options {
        let ret = av_dict_set(
            &mut dict,
            format!("{}\0", key).as_ptr() as *const libc::c_char,
            format!("{}\0", value).as_ptr() as *const libc::c_char,
            0,
        );
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

            ret.push(
                CStr::from_ptr((*opt_ptr).name)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    Ok(ret)
}

fn set_opts(ctx: *mut libc::c_void, options: HashMap<String, String>) -> Result<(), Error> {
    unsafe {
        for (key, value) in options {
            let ret = av_opt_set(
                ctx,
                format!("{}\0", key).as_ptr() as *const libc::c_char,
                format!("{}\0", value).as_ptr() as *const libc::c_char,
                AV_OPT_SEARCH_CHILDREN,
            );
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

pub use decode::*;
pub use demux::*;
pub use ffmpeg_sys_the_third;
pub use filter::*;
pub use resample::*;
pub use scale::*;
pub use stream_info::*;
