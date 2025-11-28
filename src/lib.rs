#![allow(unexpected_cfgs)]
use anyhow::Error;
use ffmpeg_sys_the_third::{
    AV_OPT_SEARCH_CHILDREN, AVDictionary, AVOption, av_dict_set, av_frame_alloc,
    av_frame_copy_props, av_hwframe_transfer_data, av_opt_next, av_opt_set, av_strerror,
};
use std::collections::HashMap;
use std::ptr;

mod audio_fifo;
mod decode;
mod demux;
mod encode;
mod filter;
mod frame;
mod mux;
mod resample;
mod scale;
mod stream_info;
mod transcode;

#[cfg(not(feature = "avcodec_version_greater_than_59_24"))]
compile_error!("avcodec version too old, < 59.24");
#[cfg(not(feature = "avutil_version_greater_than_57_24"))]
compile_error!("avutil version too old, <57.24");

#[macro_export]
macro_rules! bail_ffmpeg {
    ($x:expr) => {
        if $x < 0 {
            anyhow::bail!($crate::get_ffmpeg_error_msg($x))
        }
    };
    ($x:expr,$clean:block) => {
        if $x < 0 {
            $clean;
            anyhow::bail!($crate::get_ffmpeg_error_msg($x))
        }
    };
    ($x:expr,$msg:expr) => {
        if $x < 0 {
            anyhow::bail!(format!("{}: {}", $msg, $crate::get_ffmpeg_error_msg($x)))
        }
    };
    ($x:expr,$msg:expr,$clean:block) => {
        if $x < 0 {
            $clean;
            anyhow::bail!(format!("{}: {}", $msg, $crate::get_ffmpeg_error_msg($x)))
        }
    };
}

#[macro_export]
macro_rules! cstr {
    ($str:expr) => {
        std::ffi::CString::new($str).unwrap().into_raw()
    };
}

#[macro_export]
macro_rules! free_cstr {
    ($str:expr) => {
        #[allow(unused_unsafe)]
        let raw_str = unsafe { std::ffi::CString::from_raw($str) };
        drop(raw_str);
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

/// Version dependant [AVFrame].duration
pub fn get_frame_duration(frame: &AvFrameRef) -> i64 {
    #[cfg(feature = "avutil_version_greater_than_57_30")]
    return frame.duration;
    #[cfg(all(
        feature = "avcodec_version_greater_than_54_24",
        not(feature = "avutil_version_greater_than_57_30")
    ))]
    return frame.pkt_duration;
    #[cfg(all(
        not(feature = "avcodec_version_greater_than_54_24"),
        not(feature = "avutil_version_greater_than_57_30")
    ))]
    compile_error!("no frame duration support");
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))]
type VaList = ffmpeg_sys_the_third::va_list;
#[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
type VaList = *mut ffmpeg_sys_the_third::__va_list_tag;
#[cfg(target_os = "android")]
type VaList = [u64; 4];

pub unsafe extern "C" fn av_log_redirect(
    av_class: *mut libc::c_void,
    level: libc::c_int,
    fmt: *const libc::c_char,
    args: VaList,
) {
    unsafe {
        use ffmpeg_sys_the_third::*;
        let log_level = match level {
            AV_LOG_DEBUG => log::Level::Debug,
            AV_LOG_WARNING => log::Level::Warn,
            AV_LOG_INFO => log::Level::Info,
            AV_LOG_ERROR => log::Level::Error,
            AV_LOG_PANIC => log::Level::Error,
            AV_LOG_FATAL => log::Level::Error,
            _ => log::Level::Trace,
        };
        let mut buf: [u8; 1024] = [0; 1024];
        let mut prefix: libc::c_int = 1;
        av_log_format_line(
            av_class,
            level,
            fmt,
            args,
            buf.as_mut_ptr() as *mut libc::c_char,
            1024,
            ptr::addr_of_mut!(prefix),
        );
        #[allow(useless_ptr_null_checks)]
        let line = rstr!(buf.as_ptr() as *const libc::c_char).trim();
        log!(target: "ffmpeg", log_level, "{}", line);
    }
}

pub(crate) const AVIO_BUFFER_SIZE: usize = 4096;

pub fn get_ffmpeg_error_msg(ret: libc::c_int) -> String {
    unsafe {
        const BUF_SIZE: usize = 512;
        let mut buf: [libc::c_char; BUF_SIZE] = [0; BUF_SIZE];
        av_strerror(ret, buf.as_mut_ptr(), BUF_SIZE);
        #[allow(useless_ptr_null_checks)]
        rstr!(buf.as_ptr()).to_string()
    }
}

unsafe fn options_to_dict(options: HashMap<String, String>) -> Result<*mut AVDictionary, Error> {
    unsafe {
        let mut dict = ptr::null_mut();
        for (key, value) in options {
            let key_cstr = cstr!(key);
            let value_cstr = cstr!(value);
            let ret = av_dict_set(&mut dict, key_cstr, value_cstr, 0);
            free_cstr!(key_cstr);
            free_cstr!(value_cstr);
            bail_ffmpeg!(ret);
        }
        Ok(dict)
    }
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
            let key_cstr = cstr!(key);
            let value_cstr = cstr!(value);
            let ret = av_opt_set(ctx, key_cstr, value_cstr, AV_OPT_SEARCH_CHILDREN);
            libc::free(key_cstr as *mut libc::c_void);
            libc::free(value_cstr as *mut libc::c_void);
            bail_ffmpeg!(ret);
        }
    }
    Ok(())
}

/// Get the frame as CPU frame
pub fn get_frame_from_hw(frame: AvFrameRef) -> Result<AvFrameRef, Error> {
    if frame.hw_frames_ctx.is_null() {
        Ok(frame)
    } else {
        unsafe {
            let new_frame = av_frame_alloc();
            let ret = av_hwframe_transfer_data(new_frame, frame.ptr(), 0);
            bail_ffmpeg!(ret);
            av_frame_copy_props(new_frame, frame.ptr());
            Ok(AvFrameRef::new(new_frame))
        }
    }
}

#[cfg(test)]
pub unsafe fn generate_test_frame() -> AvFrameRef {
    unsafe {
        use ffmpeg_sys_the_third::{AVPixelFormat, av_frame_get_buffer};
        use std::mem::transmute;

        let frame = av_frame_alloc();
        (*frame).width = 1024;
        (*frame).height = 1024;
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

        AvFrameRef::new(frame)
    }
}

pub use audio_fifo::*;
pub use decode::*;
pub use demux::*;
pub use encode::*;
pub use ffmpeg_sys_the_third;
pub use filter::*;
pub use frame::*;
use log::log;
pub use mux::*;
pub use resample::*;
pub use scale::*;
pub use stream_info::*;
pub use transcode::*;
