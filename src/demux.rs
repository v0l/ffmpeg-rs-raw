use std::ffi::CStr;
use std::time::Instant;
use std::{ptr, slice};

use anyhow::Error;
use ffmpeg_sys_the_third::*;

use crate::get_ffmpeg_error_msg;
use crate::return_ffmpeg_error;
use std::fmt::{Display, Formatter};
use std::io::{Read, Write};
use std::mem::transmute;

unsafe extern "C" fn read_data(
    opaque: *mut libc::c_void,
    dst_buffer: *mut libc::c_uchar,
    size: libc::c_int,
) -> libc::c_int {
    let buffer: *mut Box<dyn Read> = opaque.cast();
    /// we loop until there is enough data to fill [size]
    let mut dst_slice: &mut [u8] = slice::from_raw_parts_mut(dst_buffer, size as usize);
    let mut w_total = 0usize;
    loop {
        return match (*buffer).read(dst_slice) {
            Ok(v) => {
                w_total += v;
                if w_total != size as usize {
                    dst_slice = &mut dst_slice[v..];
                    continue;
                }
                size
            }
            Err(e) => {
                eprintln!("read_data {}", e);
                AVERROR_EOF
            }
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DemuxerInfo {
    pub bitrate: usize,
    pub duration: f32,
    pub channels: Vec<StreamInfoChannel>,
}

unsafe impl Send for DemuxerInfo {}
unsafe impl Sync for DemuxerInfo {}

impl DemuxerInfo {
    pub fn best_stream(&self, t: StreamChannelType) -> Option<&StreamInfoChannel> {
        self.channels
            .iter()
            .filter(|a| a.channel_type == t)
            .reduce(|acc, channel| {
                if channel.best_metric() > acc.best_metric() {
                    channel
                } else {
                    acc
                }
            })
    }

    pub fn best_video(&self) -> Option<&StreamInfoChannel> {
        self.best_stream(StreamChannelType::Video)
    }

    pub fn best_audio(&self) -> Option<&StreamInfoChannel> {
        self.best_stream(StreamChannelType::Audio)
    }

    pub fn best_subtitle(&self) -> Option<&StreamInfoChannel> {
        self.best_stream(StreamChannelType::Subtitle)
    }

    pub unsafe fn is_best_stream(&self, stream: *mut AVStream) -> bool {
        match (*(*stream).codecpar).codec_type {
            AVMediaType::AVMEDIA_TYPE_VIDEO => {
                (*stream).index == self.best_video().map_or(usize::MAX, |r| r.index) as libc::c_int
            }
            AVMediaType::AVMEDIA_TYPE_AUDIO => {
                (*stream).index == self.best_audio().map_or(usize::MAX, |r| r.index) as libc::c_int
            }
            AVMediaType::AVMEDIA_TYPE_SUBTITLE => {
                (*stream).index
                    == self.best_subtitle().map_or(usize::MAX, |r| r.index) as libc::c_int
            }
            _ => false,
        }
    }
}

impl Display for DemuxerInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Demuxer Info:")?;
        for c in &self.channels {
            write!(f, "\n{}", c)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamChannelType {
    Video,
    Audio,
    Subtitle,
}

impl Display for StreamChannelType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                StreamChannelType::Video => "video",
                StreamChannelType::Audio => "audio",
                StreamChannelType::Subtitle => "subtitle",
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfoChannel {
    pub index: usize,
    pub channel_type: StreamChannelType,
    pub codec: usize,
    pub width: usize,
    pub height: usize,
    pub fps: f32,
    pub sample_rate: usize,
    pub format: usize,
}

impl StreamInfoChannel {
    pub fn best_metric(&self) -> f32 {
        match self.channel_type {
            StreamChannelType::Video => self.width as f32 * self.height as f32 * self.fps,
            StreamChannelType::Audio => self.sample_rate as f32,
            StreamChannelType::Subtitle => 999. - self.index as f32,
        }
    }
}

impl Display for StreamInfoChannel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let codec_name = unsafe { CStr::from_ptr(avcodec_get_name(transmute(self.codec as i32))) };
        write!(
            f,
            "{} #{}: codec={},size={}x{},fps={}",
            self.channel_type,
            self.index,
            codec_name.to_str().unwrap(),
            self.width,
            self.height,
            self.fps
        )
    }
}

pub struct Demuxer<'a> {
    ctx: *mut AVFormatContext,
    input: String,
    started: Instant,
    buffer: Option<Box<dyn Read + 'a>>,
}

unsafe impl Send for Demuxer<'_> {}

unsafe impl Sync for Demuxer<'_> {}

impl Demuxer<'_> {
    pub fn new(input: &str) -> Self {
        unsafe {
            let ps = avformat_alloc_context();
            Self {
                ctx: ps,
                input: input.to_string(),
                started: Instant::now(),
                buffer: None,
            }
        }
    }

    pub fn new_custom_io<T: Read + 'static>(reader: T) -> Self {
        unsafe {
            let ps = avformat_alloc_context();
            (*ps).flags |= AVFMT_FLAG_CUSTOM_IO;

            let buffer = Box::new(reader);
            Self {
                ctx: ps,
                input: String::new(),
                started: Instant::now(),
                buffer: Some(buffer),
            }
        }
    }

    unsafe fn open_input(&mut self) -> libc::c_int {
        if let Some(mut buffer) = self.buffer.take() {
            const BUFFER_SIZE: usize = 4096;
            let pb = avio_alloc_context(
                av_mallocz(BUFFER_SIZE) as *mut libc::c_uchar,
                BUFFER_SIZE as libc::c_int,
                0,
                ptr::addr_of_mut!(buffer) as _,
                Some(read_data),
                None,
                None,
            );
            (*self.ctx).pb = pb;
            avformat_open_input(
                &mut self.ctx,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } else {
            avformat_open_input(
                &mut self.ctx,
                format!("{}\0", self.input).as_ptr() as *const libc::c_char,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        }
    }

    pub unsafe fn probe_input(&mut self) -> Result<DemuxerInfo, Error> {
        let ret = self.open_input();
        return_ffmpeg_error!(ret);

        if avformat_find_stream_info(self.ctx, ptr::null_mut()) < 0 {
            return Err(Error::msg("Could not find stream info"));
        }
        av_dump_format(self.ctx, 0, ptr::null_mut(), 0);

        let mut channel_infos = vec![];

        for n in 0..(*self.ctx).nb_streams as usize {
            let stream = *(*self.ctx).streams.add(n);
            match (*(*stream).codecpar).codec_type {
                AVMediaType::AVMEDIA_TYPE_VIDEO => {
                    channel_infos.push(StreamInfoChannel {
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

impl Drop for Demuxer<'_> {
    fn drop(&mut self) {
        unsafe {
            avformat_free_context(self.ctx);
            self.ctx = ptr::null_mut();
        }
    }
}
