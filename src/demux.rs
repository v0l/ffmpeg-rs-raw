use crate::{bail_ffmpeg, cstr, rstr};
use crate::{DemuxerInfo, StreamInfo, StreamType};
#[cfg(feature = "avformat_version_greater_than_60_19")]
use crate::{StreamGroupInfo, StreamGroupType};
use anyhow::{bail, Error, Result};
#[cfg(feature = "avformat_version_greater_than_60_22")]
use ffmpeg_sys_the_third::AVStreamGroupParamsType::AV_STREAM_GROUP_PARAMS_TILE_GRID;
use ffmpeg_sys_the_third::*;
use log::warn;
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
    match buffer.read(dst_slice) {
        Ok(r) => r as libc::c_int,
        Err(e) => {
            eprintln!("read_data {}", e);
            AVERROR_EOF
        }
    }
}

pub enum DemuxerInput {
    Url(String),
    Reader(Option<SlimBox<dyn Read>>, Option<String>),
}

pub struct Demuxer {
    ctx: *mut AVFormatContext,
    input: DemuxerInput,
}

impl Demuxer {
    /// Create a new [Demuxer] from a file path or url
    pub fn new(input: &str) -> Result<Self> {
        unsafe {
            let ctx = avformat_alloc_context();
            if ctx.is_null() {
                bail!("Failed to allocate AV context");
            }
            Ok(Self {
                ctx,
                input: DemuxerInput::Url(input.to_string()),
            })
        }
    }

    /// Create a new [Demuxer] from an object that implements [Read]
    pub fn new_custom_io<R: Read + 'static>(reader: R, url: Option<String>) -> Result<Self> {
        unsafe {
            let ctx = avformat_alloc_context();
            if ctx.is_null() {
                bail!("Failed to allocate AV context");
            }
            (*ctx).flags |= AVFMT_FLAG_CUSTOM_IO;

            Ok(Self {
                ctx,
                input: DemuxerInput::Reader(Some(slimbox_unsize!(reader)), url),
            })
        }
    }

    /// Set [AVFormatContext] options
    pub fn set_opt(&mut self, options: HashMap<String, String>) -> Result<(), Error> {
        crate::set_opts(self.ctx as *mut libc::c_void, options)
    }

    unsafe fn open(&mut self) -> Result<()> {
        match &mut self.input {
            DemuxerInput::Url(input) => {
                let ret = avformat_open_input(
                    &mut self.ctx,
                    cstr!(input.as_str()),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                bail_ffmpeg!(ret);
                Ok(())
            }
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
                if pb.is_null() {
                    bail!("failed to allocate avio context");
                }

                (*self.ctx).pb = pb;
                let ret = avformat_open_input(
                    &mut self.ctx,
                    if let Some(url) = url {
                        cstr!(url.as_str())
                    } else {
                        ptr::null_mut()
                    },
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                bail_ffmpeg!(ret);
                Ok(())
            }
        }
    }

    pub unsafe fn probe_input(&mut self) -> Result<DemuxerInfo, Error> {
        self.open()?;
        if avformat_find_stream_info(self.ctx, ptr::null_mut()) < 0 {
            return Err(Error::msg("Could not find stream info"));
        }

        let mut streams = vec![];
        #[cfg(feature = "avformat_version_greater_than_60_19")]
        let mut stream_groups = vec![];

        let mut n_stream = 0;
        #[cfg(feature = "avformat_version_greater_than_60_19")]
        for n in 0..(*self.ctx).nb_stream_groups as usize {
            let group = *(*self.ctx).stream_groups.add(n);
            n_stream += (*group).nb_streams as usize;
            match (*group).type_ {
                #[cfg(feature = "avformat_version_greater_than_60_22")]
                AV_STREAM_GROUP_PARAMS_TILE_GRID => {
                    let tg = (*group).params.tile_grid;
                    let codec_par = (*(*(*group).streams.add(0))).codecpar;
                    stream_groups.push(StreamGroupInfo {
                        index: (*group).index as usize,
                        group_type: StreamGroupType::TileGrid {
                            tiles: (*tg).nb_tiles as usize,
                            width: (*tg).width as usize,
                            height: (*tg).height as usize,
                            format: (*codec_par).format as isize,
                            codec: (*codec_par).codec_id as isize,
                        },
                        group,
                    });
                }
                t => {
                    warn!(
                        "Unsupported stream group type {}, skipping.",
                        rstr!(avformat_stream_group_name(t))
                    );
                }
            }
        }
        while n_stream < (*self.ctx).nb_streams as usize {
            let stream = *(*self.ctx).streams.add(n_stream);
            n_stream += 1;
            let lang = av_dict_get((*stream).metadata, cstr!("language"), ptr::null_mut(), 0);
            let language = if lang.is_null() {
                "".to_string()
            } else {
                rstr!((*lang).value).to_string()
            };
            match (*(*stream).codecpar).codec_type {
                AVMediaType::AVMEDIA_TYPE_VIDEO => {
                    streams.push(StreamInfo {
                        stream,
                        index: (*stream).index as usize,
                        codec: (*(*stream).codecpar).codec_id as isize,
                        stream_type: StreamType::Video,
                        width: (*(*stream).codecpar).width as usize,
                        height: (*(*stream).codecpar).height as usize,
                        fps: av_q2d((*stream).avg_frame_rate) as f32,
                        format: (*(*stream).codecpar).format as isize,
                        sample_rate: 0,
                        language,
                    });
                }
                AVMediaType::AVMEDIA_TYPE_AUDIO => {
                    streams.push(StreamInfo {
                        stream,
                        index: (*stream).index as usize,
                        codec: (*(*stream).codecpar).codec_id as isize,
                        stream_type: StreamType::Audio,
                        width: (*(*stream).codecpar).width as usize,
                        height: (*(*stream).codecpar).height as usize,
                        fps: 0.0,
                        format: (*(*stream).codecpar).format as isize,
                        sample_rate: (*(*stream).codecpar).sample_rate as usize,
                        language,
                    });
                }
                AVMediaType::AVMEDIA_TYPE_SUBTITLE => {
                    streams.push(StreamInfo {
                        stream,
                        index: (*stream).index as usize,
                        codec: (*(*stream).codecpar).codec_id as isize,
                        stream_type: StreamType::Subtitle,
                        width: 0,
                        height: 0,
                        fps: 0.0,
                        format: 0,
                        sample_rate: 0,
                        language,
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
            format: rstr!((*(*self.ctx).iformat).name).to_string(),
            mime_types: rstr!((*(*self.ctx).iformat).mime_type).to_string(),
            streams,
            #[cfg(feature = "avformat_version_greater_than_60_19")]
            groups: stream_groups,
        };
        Ok(info)
    }

    pub unsafe fn get_packet(&mut self) -> Result<(*mut AVPacket, *mut AVStream), Error> {
        let mut pkt = av_packet_alloc();
        let ret = av_read_frame(self.ctx, pkt);
        if ret == AVERROR_EOF {
            av_packet_free(&mut pkt);
            return Ok((ptr::null_mut(), ptr::null_mut()));
        }
        bail_ffmpeg!(ret);

        let stream = *(*self.ctx).streams.add((*pkt).stream_index as usize);
        (*pkt).time_base = (*stream).time_base;
        let pkg = (pkt, stream);
        Ok(pkg)
    }
}

impl Drop for Demuxer {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                match self.input {
                    DemuxerInput::Reader(_, _) => {
                        av_free((*(*self.ctx).pb).buffer as *mut _);
                        drop(SlimBox::<dyn Read>::from_raw((*(*self.ctx).pb).opaque));
                        avio_context_free(&mut (*self.ctx).pb);
                    }
                    _ => {}
                }
                avformat_close_input(&mut self.ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "avformat_version_greater_than_60_19")]
    #[test]
    #[ignore]
    fn test_stream_groups() -> Result<()> {
        unsafe {
            let mut demux =
                Demuxer::new("https://trac.ffmpeg.org/raw-attachment/ticket/11170/IMG_4765.HEIC")?;
            let probe = demux.probe_input()?;
            assert_eq!(3, probe.streams.len());
            assert_eq!(0, probe.groups.len());
            assert!(matches!(
                probe.groups[0].group_type,
                StreamGroupType::TileGrid { .. }
            ));
        }
        Ok(())
    }

    /// Test for leaking file handles
    #[test]
    fn probe_lots() -> Result<()> {
        rlimit::setrlimit(rlimit::Resource::NOFILE, 64, 128)?;

        let nof_limit = rlimit::Resource::NOFILE.get_hard()?;
        for n in 0..nof_limit {
            let mut demux = Demuxer::new("./test_output/test.png")?;
            unsafe {
                if let Err(e) = demux.probe_input() {
                    bail!("Failed on {}: {}", n, e);
                }
            }
        }
        Ok(())
    }
}
