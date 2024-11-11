use crate::{bail_ffmpeg, cstr, rstr, StreamGroupInfo, StreamGroupType};
use crate::{DemuxerInfo, StreamInfo, StreamType};
use anyhow::{bail, Error, Result};
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
        bail_ffmpeg!(ret);

        if avformat_find_stream_info(self.ctx, ptr::null_mut()) < 0 {
            return Err(Error::msg("Could not find stream info"));
        }

        let mut streams = vec![];
        let mut stream_groups = vec![];

        let mut n_stream = 0;
        for n in 0..(*self.ctx).nb_stream_groups as usize {
            let group = *(*self.ctx).stream_groups.add(n);
            n_stream += (*group).nb_streams as usize;
            match (*group).type_ {
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
            streams,
            groups: stream_groups,
        };
        Ok(info)
    }

    pub unsafe fn get_packet(&mut self) -> Result<(*mut AVPacket, *mut AVStream), Error> {
        let pkt: *mut AVPacket = av_packet_alloc();
        let ret = av_read_frame(self.ctx, pkt);
        if ret == AVERROR_EOF {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_groups() -> Result<()> {
        unsafe {
            let mut demux =
                Demuxer::new("/core/Camera/Syncthing/Camera S22/Camera/20241104_121607.heic")?;
            let probe = demux.probe_input()?;
            assert_eq!(1, probe.streams.len());
            assert_eq!(1, probe.groups.len());
            assert!(matches!(
                probe.groups[0].group_type,
                StreamGroupType::TileGrid { .. }
            ));
        }
        Ok(())
    }
}
