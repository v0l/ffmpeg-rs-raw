use ffmpeg_sys_the_third::{avcodec_get_name, AVMediaType, AVStream};
use std::ffi::CStr;
use std::fmt::{Display, Formatter};
use std::intrinsics::transmute;

#[derive(Clone, Debug, PartialEq)]
pub struct DemuxerInfo {
    pub bitrate: usize,
    pub duration: f32,
    pub channels: Vec<StreamInfoChannel>,
}

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

    // private stream pointer
    pub(crate) stream: *mut AVStream,
}

unsafe impl Send for StreamInfoChannel {}
unsafe impl Sync for StreamInfoChannel {}

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