use crate::{AvPacketRef, DemuxerInfo, StreamInfo, StreamType};
#[cfg(feature = "avformat_version_greater_than_60_19")]
use crate::{StreamGroupInfo, StreamGroupType};
use crate::{bail_ffmpeg, cstr, rstr};
use anyhow::{Error, Result, bail};
#[cfg(feature = "avformat_version_greater_than_60_22")]
use ffmpeg_sys_the_third::AVStreamGroupParamsType::AV_STREAM_GROUP_PARAMS_TILE_GRID;
use ffmpeg_sys_the_third::*;
use log::warn;
use slimbox::{SlimBox, SlimMut, slimbox_unsize};
use std::collections::HashMap;
use std::io::Read;
use std::{ptr, slice};

#[unsafe(no_mangle)]
extern "C" fn read_data(
    opaque: *mut libc::c_void,
    dst_buffer: *mut libc::c_uchar,
    size: libc::c_int,
) -> libc::c_int {
    let mut buffer: SlimMut<'_, dyn Read + 'static> = unsafe { SlimMut::from_raw(opaque) };
    let dst_slice: &mut [u8] = unsafe { slice::from_raw_parts_mut(dst_buffer, size as usize) };
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
    buffer_size: usize,
    format: Option<String>,
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
                buffer_size: 4096,
                format: None,
            })
        }
    }

    /// Configure the buffer size for custom_io
    pub fn with_buffer_size(mut self, buffer_size: usize) -> Self {
        self.set_buffer_size(buffer_size);
        self
    }

    /// Configure the buffer size for custom_io
    pub fn set_buffer_size(&mut self, buffer_size: usize) {
        self.buffer_size = buffer_size;
    }

    /// Configure format hints for demuxer
    pub fn with_format(mut self, format: &str) -> Self {
        self.format = Some(format.to_string());
        self
    }

    /// Configure format hints for demuxer
    pub fn set_format(&mut self, format: &str) {
        self.format = Some(format.to_string());
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
                buffer_size: 1024 * 16,
                format: None,
            })
        }
    }

    /// Set [AVFormatContext] options
    pub fn set_opt(&mut self, options: HashMap<String, String>) -> Result<(), Error> {
        crate::set_opts(self.ctx as *mut libc::c_void, options)
    }

    pub fn context(&self) -> *mut AVFormatContext {
        self.ctx
    }

    unsafe fn open(&mut self) -> Result<()> {
        unsafe {
            let format = if let Some(f) = &self.format {
                let fmt_str = cstr!(f.as_str());
                let ret = av_find_input_format(fmt_str);
                libc::free(fmt_str as *mut libc::c_void);
                if ret.is_null() {
                    bail!("Input format {} not found", f);
                }
                ret
            } else {
                ptr::null()
            };

            match &mut self.input {
                DemuxerInput::Url(input) => {
                    let input_cstr = cstr!(input.as_str());
                    let ret =
                        avformat_open_input(&mut self.ctx, input_cstr, format, ptr::null_mut());
                    libc::free(input_cstr as *mut libc::c_void);
                    bail_ffmpeg!(ret);
                    Ok(())
                }
                DemuxerInput::Reader(input, url) => {
                    let input = input.take().expect("input stream already taken");
                    let pb = avio_alloc_context(
                        av_mallocz(self.buffer_size) as *mut _,
                        self.buffer_size as _,
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
                    let url_cstr = if let Some(url) = url {
                        cstr!(url.as_str())
                    } else {
                        ptr::null_mut()
                    };
                    let ret = avformat_open_input(&mut self.ctx, url_cstr, format, ptr::null_mut());
                    if !url_cstr.is_null() {
                        libc::free(url_cstr as *mut libc::c_void);
                    }
                    bail_ffmpeg!(ret);
                    Ok(())
                }
            }
        }
    }

    pub unsafe fn probe_input(&mut self) -> Result<DemuxerInfo, Error> {
        unsafe {
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
                let lang_key = cstr!("language");
                let lang = av_dict_get((*stream).metadata, lang_key, ptr::null_mut(), 0);
                libc::free(lang_key as *mut libc::c_void);
                let language = if lang.is_null() {
                    "".to_string()
                } else {
                    rstr!((*lang).value).to_string()
                };
                let q = av_q2d((*stream).time_base);
                match (*(*stream).codecpar).codec_type {
                    AVMediaType::AVMEDIA_TYPE_VIDEO => {
                        streams.push(StreamInfo {
                            stream,
                            index: (*stream).index as _,
                            codec: (*(*stream).codecpar).codec_id as _,
                            stream_type: StreamType::Video,
                            width: (*(*stream).codecpar).width as _,
                            height: (*(*stream).codecpar).height as _,
                            fps: av_q2d((*stream).avg_frame_rate) as _,
                            format: (*(*stream).codecpar).format as _,
                            sample_rate: 0,
                            channels: 0,
                            start_time: ((*stream).start_time as f64 * q) as f32,
                            language,
                            timebase: ((*stream).time_base.num, (*stream).time_base.den),
                        });
                    }
                    AVMediaType::AVMEDIA_TYPE_AUDIO => {
                        streams.push(StreamInfo {
                            stream,
                            index: (*stream).index as _,
                            codec: (*(*stream).codecpar).codec_id as _,
                            stream_type: StreamType::Audio,
                            width: (*(*stream).codecpar).width as _,
                            height: (*(*stream).codecpar).height as _,
                            fps: 0.0,
                            format: (*(*stream).codecpar).format as _,
                            sample_rate: (*(*stream).codecpar).sample_rate as _,
                            channels: (*(*stream).codecpar).ch_layout.nb_channels as _,
                            start_time: ((*stream).start_time as f64 * q) as f32,
                            language,
                            timebase: ((*stream).time_base.num, (*stream).time_base.den),
                        });
                    }
                    AVMediaType::AVMEDIA_TYPE_SUBTITLE => {
                        streams.push(StreamInfo {
                            stream,
                            index: (*stream).index as _,
                            codec: (*(*stream).codecpar).codec_id as _,
                            stream_type: StreamType::Subtitle,
                            width: 0,
                            height: 0,
                            fps: 0.0,
                            format: 0,
                            sample_rate: 0,
                            channels: 0,
                            start_time: ((*stream).start_time as f64 * q) as f32,
                            language,
                            timebase: ((*stream).time_base.num, (*stream).time_base.den),
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
    }

    pub unsafe fn get_packet(&mut self) -> Result<(Option<AvPacketRef>, *mut AVStream), Error> {
        unsafe {
            let pkt = av_packet_alloc();
            let ret = av_read_frame(self.ctx, pkt);
            if ret == AVERROR_EOF {
                av_packet_free(&mut (pkt as *mut _));
                return Ok((None, ptr::null_mut()));
            }
            bail_ffmpeg!(ret);

            let stream = self.get_stream((*pkt).stream_index as _)?;
            (*pkt).time_base = (*stream).time_base;
            Ok((Some(AvPacketRef::new(pkt)), stream))
        }
    }

    /// Get stream by index from context
    pub unsafe fn get_stream(&mut self, index: usize) -> Result<*mut AVStream, Error> {
        unsafe {
            if self.ctx.is_null() {
                bail!("context is null")
            }
            if index >= (*self.ctx).nb_streams as _ {
                bail!("Invalid stream index")
            }
            Ok(*(*self.ctx).streams.add(index))
        }
    }
}

impl Drop for Demuxer {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                if let DemuxerInput::Reader(_, _) = self.input {
                    let mut io = (*self.ctx).pb;
                    if !io.is_null() {
                        av_freep((*io).buffer as *mut _);
                        drop(SlimBox::<dyn Read>::from_raw((*io).opaque));
                        avio_context_free(&mut io);
                    }
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
            demux.set_format("image2");
            unsafe {
                if let Err(e) = demux.probe_input() {
                    bail!("Failed on {}: {}", n, e);
                }
            }
        }
        Ok(())
    }
}
