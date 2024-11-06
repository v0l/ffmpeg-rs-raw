use crate::{cstr, return_ffmpeg_error};
use crate::{get_ffmpeg_error_msg, DemuxerInfo, StreamChannelType, StreamInfoChannel};
use anyhow::Error;
use ffmpeg_sys_the_third::*;
use slimbox::{slimbox_unsize, SlimBox, SlimMut};
use std::collections::HashMap;
use std::io::Read;
use std::{ptr, slice};

#[no_mangle]
unsafe extern "C" fn read_data(
    opaque: *mut libc::c_void,
    dst_buffer: *mut libc::c_uchar,
    size: libc::c_int,
) -> libc::c_int {
    let mut buffer: SlimMut<'_, dyn Read + 'static> = SlimMut::from_raw(opaque);
    let dst_slice: &mut [u8] = slice::from_raw_parts_mut(dst_buffer, size as usize);
    match buffer.read_exact(dst_slice) {
        Ok(_) => size,
        Err(e) => {
            eprintln!("read_data {}", e);
            AVERROR_EOF
        }
    }
}

pub enum DemuxerInput {
    Url(String),
    Reader(Option<SlimBox<dyn Read + 'static>>, Option<String>),
}

pub struct Demuxer {
    ctx: *mut AVFormatContext,
    input: DemuxerInput,
}

impl Demuxer {
    /// Create a new [Demuxer] from a file path or url
    pub fn new(input: &str) -> Self {
        unsafe {
            let ctx = avformat_alloc_context();
            Self {
                ctx,
                input: DemuxerInput::Url(input.to_string()),
            }
        }
    }

    /// Create a new [Demuxer] from an object that implements [Read]
    pub fn new_custom_io<R: Read + 'static>(reader: R, url: Option<String>) -> Self {
        unsafe {
            let ctx = avformat_alloc_context();
            (*ctx).flags |= AVFMT_FLAG_CUSTOM_IO;

            Self {
                ctx,
                input: DemuxerInput::Reader(Some(slimbox_unsize!(reader)), url),
            }
        }
    }

    /// Set [AVFormatContext] options
    pub fn set_opt(&mut self, options: HashMap<String, String>) -> Result<(), Error> {
        crate::set_opts(self.ctx as *mut libc::c_void, options)
    }

    unsafe fn open_input(&mut self) -> libc::c_int {
        match &mut self.input {
            DemuxerInput::Url(input) => avformat_open_input(
                &mut self.ctx,
                cstr!(input),
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            DemuxerInput::Reader(input, url) => {
                let input = input.take().expect("input stream already taken");
                const BUFFER_SIZE: usize = 4096;
                let pb = avio_alloc_context(
                    av_mallocz(BUFFER_SIZE) as *mut libc::c_uchar,
                    BUFFER_SIZE as libc::c_int,
                    0,
                    input.into_raw(),
                    Some(read_data),
                    None,
                    None,
                );

                (*self.ctx).pb = pb;
                avformat_open_input(
                    &mut self.ctx,
                    if let Some(url) = url {
                        cstr!(url)
                    } else {
                        ptr::null_mut()
                    },
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            }
        }
    }

    pub unsafe fn probe_input(&mut self) -> Result<DemuxerInfo, Error> {
        let ret = self.open_input();
        return_ffmpeg_error!(ret);

        if avformat_find_stream_info(self.ctx, ptr::null_mut()) < 0 {
            return Err(Error::msg("Could not find stream info"));
        }

        let mut channel_infos = vec![];

        for n in 0..(*self.ctx).nb_streams as usize {
            let stream = *(*self.ctx).streams.add(n);
            match (*(*stream).codecpar).codec_type {
                AVMediaType::AVMEDIA_TYPE_VIDEO => {
                    channel_infos.push(StreamInfoChannel {
                        stream,
                        index: (*stream).index as usize,
                        codec: (*(*stream).codecpar).codec_id as usize,
                        channel_type: StreamChannelType::Video,
                        width: (*(*stream).codecpar).width as usize,
                        height: (*(*stream).codecpar).height as usize,
                        fps: av_q2d((*stream).avg_frame_rate) as f32,
                        format: (*(*stream).codecpar).format as usize,
                        sample_rate: 0,
                    });
                }
                AVMediaType::AVMEDIA_TYPE_AUDIO => {
                    channel_infos.push(StreamInfoChannel {
                        stream,
                        index: (*stream).index as usize,
                        codec: (*(*stream).codecpar).codec_id as usize,
                        channel_type: StreamChannelType::Audio,
                        width: (*(*stream).codecpar).width as usize,
                        height: (*(*stream).codecpar).height as usize,
                        fps: 0.0,
                        format: (*(*stream).codecpar).format as usize,
                        sample_rate: (*(*stream).codecpar).sample_rate as usize,
                    });
                }
                AVMediaType::AVMEDIA_TYPE_SUBTITLE => {
                    channel_infos.push(StreamInfoChannel {
                        stream,
                        index: (*stream).index as usize,
                        codec: (*(*stream).codecpar).codec_id as usize,
                        channel_type: StreamChannelType::Subtitle,
                        width: 0,
                        height: 0,
                        fps: 0.0,
                        format: 0,
                        sample_rate: 0,
                    });
                }
                AVMediaType::AVMEDIA_TYPE_ATTACHMENT => {}
                AVMediaType::AVMEDIA_TYPE_NB => {}
                _ => {}
            }
        }

        let info = DemuxerInfo {
            duration: (*self.ctx).duration as f32 / AV_TIME_BASE as f32,
            bitrate: (*self.ctx).bit_rate as usize,
            channels: channel_infos,
        };
        Ok(info)
    }

    pub unsafe fn get_packet(&mut self) -> Result<(*mut AVPacket, *mut AVStream), Error> {
        let pkt: *mut AVPacket = av_packet_alloc();
        let ret = av_read_frame(self.ctx, pkt);
        if ret == AVERROR_EOF {
            return Ok((ptr::null_mut(), ptr::null_mut()));
        }
        if ret < 0 {
            let msg = get_ffmpeg_error_msg(ret);
            return Err(Error::msg(msg));
        }
        let stream = *(*self.ctx).streams.add((*pkt).stream_index as usize);
        let pkg = (pkt, stream);
        Ok(pkg)
    }
}

impl Drop for Demuxer {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe {
                if let DemuxerInput::Reader(_, _) = self.input {
                    drop(SlimBox::<dyn Read>::from_raw((*(*self.ctx).pb).opaque));
                }
                avformat_free_context(self.ctx);
                self.ctx = ptr::null_mut();
            }
        }
    }
}
