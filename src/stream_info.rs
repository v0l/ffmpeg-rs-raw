use crate::{format_time, rstr};
use ffmpeg_sys_the_third::{
    av_get_pix_fmt_name, av_get_sample_fmt_name, avcodec_get_name, AVMediaType, AVStream,
    AVStreamGroup,
};
use std::fmt::{Display, Formatter};
use std::intrinsics::transmute;

#[derive(Clone, Debug, PartialEq)]
pub struct DemuxerInfo {
    pub bitrate: usize,
    pub duration: f32,
    pub streams: Vec<StreamInfo>,
    pub groups: Vec<StreamGroupInfo>,
}

impl DemuxerInfo {
    pub fn best_stream(&self, t: StreamType) -> Option<&StreamInfo> {
        self.streams
            .iter()
            .filter(|a| a.stream_type == t)
            .reduce(|acc, channel| {
                if channel.best_metric() > acc.best_metric() {
                    channel
                } else {
                    acc
                }
            })
    }

    pub fn best_video(&self) -> Option<&StreamInfo> {
        self.best_stream(StreamType::Video)
    }

    pub fn best_audio(&self) -> Option<&StreamInfo> {
        self.best_stream(StreamType::Audio)
    }

    pub fn best_subtitle(&self) -> Option<&StreamInfo> {
        self.best_stream(StreamType::Subtitle)
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
        let bitrate_str = if self.bitrate > 1_000_000 {
            format!("{}M", self.bitrate / 1_000_000)
        } else if self.bitrate > 1_000 {
            format!("{}k", self.bitrate / 1_000)
        } else {
            self.bitrate.to_string()
        };

        write!(
            f,
            "Demuxer Info: duration={}, bitrate={}",
            format_time(self.duration),
            bitrate_str
        )?;
        for c in &self.streams {
            write!(f, "\n  {}", c)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamType {
    Video,
    Audio,
    Subtitle,
}

impl Display for StreamType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                StreamType::Video => "video",
                StreamType::Audio => "audio",
                StreamType::Subtitle => "subtitle",
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub index: usize,
    pub stream_type: StreamType,
    pub codec: isize,
    pub format: isize,

    pub width: usize,
    pub height: usize,

    pub fps: f32,
    pub sample_rate: usize,

    // private stream pointer
    pub(crate) stream: *mut AVStream,
}

unsafe impl Send for StreamInfo {}

impl StreamInfo {
    pub fn best_metric(&self) -> f32 {
        match self.stream_type {
            StreamType::Video => self.width as f32 * self.height as f32 * self.fps,
            StreamType::Audio => self.sample_rate as f32,
            StreamType::Subtitle => 999. - self.index as f32,
        }
    }
}

impl Display for StreamInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let codec_name = unsafe { rstr!(avcodec_get_name(transmute(self.codec as i32))) };
        match self.stream_type {
            StreamType::Video => write!(
                f,
                "{} #{}: codec={},size={}x{},fps={:.3},pix_fmt={}",
                self.stream_type,
                self.index,
                codec_name,
                self.width,
                self.height,
                self.fps,
                unsafe { rstr!(av_get_pix_fmt_name(transmute(self.format as libc::c_int))) },
            ),
            StreamType::Audio => write!(
                f,
                "{} #{}: codec={},format={},sample_rate={}",
                self.stream_type,
                self.index,
                codec_name,
                unsafe {
                    rstr!(av_get_sample_fmt_name(transmute(
                        self.format as libc::c_int,
                    )))
                },
                self.sample_rate,
            ),
            StreamType::Subtitle => write!(
                f,
                "{} #{}: codec={}",
                self.stream_type, self.index, codec_name
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamGroupType {
    TileGrid {
        tiles: usize,
        width: usize,
        height: usize,
        codec: isize,
        format: isize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamGroupInfo {
    pub index: usize,
    pub group_type: StreamGroupType,

    // private pointer
    pub(crate) group: *mut AVStreamGroup,
}

unsafe impl Send for StreamGroupInfo {}
