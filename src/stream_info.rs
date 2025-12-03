#[cfg(feature = "avformat_version_greater_than_60_19")]
use ffmpeg_sys_the_third::AVStreamGroup;
use ffmpeg_sys_the_third::{
    AVMediaType, AVStream,
};

use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, PartialEq)]
pub struct DemuxerInfo {
    /// Average bitrate of the media
    pub bitrate: usize,
    /// Duration of the media in seconds
    pub duration: f32,
    /// Comma separated list of formats supported by the demuxer
    pub format: String,
    /// Comma separated list of mime-types used during probing
    pub mime_types: String,
    /// List of streams contained in the media
    pub streams: Vec<StreamInfo>,
    #[cfg(feature = "avformat_version_greater_than_60_19")]
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
        unsafe {
            match (*(*stream).codecpar).codec_type {
                AVMediaType::AVMEDIA_TYPE_VIDEO => {
                    (*stream).index
                        == self.best_video().map_or(usize::MAX, |r| r.index) as libc::c_int
                }
                AVMediaType::AVMEDIA_TYPE_AUDIO => {
                    (*stream).index
                        == self.best_audio().map_or(usize::MAX, |r| r.index) as libc::c_int
                }
                AVMediaType::AVMEDIA_TYPE_SUBTITLE => {
                    (*stream).index
                        == self.best_subtitle().map_or(usize::MAX, |r| r.index) as libc::c_int
                }
                _ => false,
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum StreamType {
    #[default]
    Unknown,
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
                StreamType::Unknown => "Unknown",
                StreamType::Video => "video",
                StreamType::Audio => "audio",
                StreamType::Subtitle => "subtitle",
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StreamInfo {
    /// Stream index
    pub index: usize,
    /// Stream type video/audio/subtitle
    pub stream_type: StreamType,
    /// Stream codec
    pub codec: isize,
    /// Pixel format / Sample format
    pub format: isize,
    /// Average bitrate of the stream
    pub bitrate: usize,

    /// Video width
    pub width: usize,
    /// Video height
    pub height: usize,
    /// Video FPS
    pub fps: f32,
    /// Start time offset
    pub start_time: f32,
    /// Codec profile (h264)
    pub profile: isize,
    /// Codec level (h264)
    pub level: isize,
    /// Color space for video frames
    pub color_space: isize,
    /// Color range for video frames
    pub color_range: isize,

    /// Audio sample rate
    pub sample_rate: usize,
    /// Subtitle / Audio language
    pub language: String,
    /// Number of audio channels
    pub channels: u8,
    /// Stream timebase (num, den)
    pub timebase: (i32, i32),

    // private stream pointer
    pub(crate) stream: *mut AVStream,
}

unsafe impl Send for StreamInfo {}

impl StreamInfo {
    pub fn best_metric(&self) -> f32 {
        match self.stream_type {
            StreamType::Unknown => 0.0,
            StreamType::Video => self.width as f32 * self.height as f32 * self.fps,
            StreamType::Audio => self.sample_rate as f32,
            StreamType::Subtitle => 999. - self.index as f32,
        }
    }
}

#[cfg(feature = "avformat_version_greater_than_60_19")]
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

#[cfg(feature = "avformat_version_greater_than_60_19")]
#[derive(Clone, Debug, PartialEq)]
pub struct StreamGroupInfo {
    pub index: usize,
    pub group_type: StreamGroupType,

    // private pointer
    pub(crate) group: *mut AVStreamGroup,
}

#[cfg(feature = "avformat_version_greater_than_60_19")]
unsafe impl Send for StreamGroupInfo {}
