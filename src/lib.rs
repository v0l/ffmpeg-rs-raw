use ffmpeg_sys_the_third::av_make_error_string;
use std::ffi::CStr;

mod decode;
mod demux;
mod resample;
mod scale;

#[macro_export(crate)]
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

pub use decode::*;
pub use demux::*;
pub use resample::*;
pub use scale::*;
