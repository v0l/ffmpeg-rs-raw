use anyhow::Error;
use ffmpeg_sys_the_third::{
    av_dict_set, av_make_error_string, av_opt_next, av_opt_set, AVDictionary, AVOption,
    AV_OPT_SEARCH_CHILDREN,
};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr;

mod decode;
mod demux;
mod resample;
mod scale;
mod stream_info;

#[macro_export]
macro_rules! return_ffmpeg_error {
    ($x:expr) => {
        if $x < 0 {
            return Err(Error::msg(get_ffmpeg_error_msg($x)));
        }
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

pub use decode::*;
pub use demux::*;
pub use ffmpeg_sys_the_third;
pub use resample::*;
pub use scale::*;
pub use stream_info::*;
