use crate::{bail_ffmpeg, cstr, set_opts, Encoder, AVIO_BUFFER_SIZE};
use anyhow::{bail, Result};
use ffmpeg_sys_the_third::{
    av_free, av_interleaved_write_frame, av_mallocz, av_packet_rescale_ts, av_write_trailer,
    avcodec_parameters_copy, avcodec_parameters_from_context, avformat_alloc_output_context2,
    avformat_free_context, avformat_new_stream, avformat_write_header, avio_alloc_context,
    avio_open, AVFormatContext, AVIOContext, AVPacket, AVStream, AVERROR_EOF, AVFMT_GLOBALHEADER,
    AVFMT_NOFILE, AVIO_FLAG_DIRECT, AVIO_FLAG_WRITE, AV_CODEC_FLAG_GLOBAL_HEADER,
};
use slimbox::{slimbox_unsize, SlimBox, SlimMut};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::{ptr, slice};

unsafe extern "C" fn write_data<T>(
    opaque: *mut libc::c_void,
    buffer: *const u8,
    size: libc::c_int,
) -> libc::c_int
where
    T: Write + 'static + ?Sized,
{
    let mut writer: SlimMut<'_, T> = SlimMut::from_raw(opaque);
    let data = slice::from_raw_parts(buffer, size as usize);
    match writer.write_all(data) {
        Ok(_) => size,
        Err(e) => {
            eprintln!("write_data {}", e);
            AVERROR_EOF
        }
    }
}

unsafe extern "C" fn seek_data(opaque: *mut libc::c_void, offset: i64, whence: libc::c_int) -> i64 {
    let mut writer: SlimMut<'_, dyn WriteSeek + 'static> = SlimMut::from_raw(opaque);
    match whence {
        libc::SEEK_SET => writer.seek(SeekFrom::Start(offset as u64)).unwrap_or(0) as i64,
        libc::SEEK_CUR => writer.seek(SeekFrom::Current(offset)).unwrap_or(0) as i64,
        libc::SEEK_END => writer.seek(SeekFrom::End(offset)).unwrap_or(0) as i64,
        _ => panic!("seek_data not supported from whence {}", whence),
    }
}

pub struct Muxer {
    ctx: *mut AVFormatContext,
    output: MuxerOutput,
}

pub trait WriteSeek: Seek + Write {}
impl<T: Seek + Write> WriteSeek for T {}

pub enum MuxerOutput {
    Url(String),
    WriterSeeker(Option<SlimBox<dyn WriteSeek + 'static>>),
    Writer(Option<SlimBox<dyn Write + 'static>>),
}

impl TryInto<*mut AVIOContext> for &mut MuxerOutput {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<*mut AVIOContext, Self::Error> {
        unsafe {
            Ok(match self {
                MuxerOutput::Writer(ref mut w) => {
                    let writer = w.take().expect("writer already consumed");
                    let pb = avio_alloc_context(
                        av_mallocz(AVIO_BUFFER_SIZE) as *mut _,
                        AVIO_BUFFER_SIZE as _,
                        1,
                        writer.into_raw(),
                        None,
                        Some(write_data::<dyn Write + 'static>),
                        None,
                    );

                    if pb.is_null() {
                        bail!("failed to allocate AVIO from MuxerOutput");
                    }
                    pb
                }
                MuxerOutput::WriterSeeker(ref mut w) => {
                    let writer = w.take().expect("writer already consumed");
                    let pb = avio_alloc_context(
                        av_mallocz(AVIO_BUFFER_SIZE) as *mut _,
                        AVIO_BUFFER_SIZE as _,
                        1,
                        writer.into_raw(),
                        None,
                        Some(write_data::<dyn WriteSeek + 'static>),
                        Some(seek_data),
                    );

                    if pb.is_null() {
                        bail!("failed to allocate AVIO from MuxerOutput");
                    }
                    pb
                }
                MuxerOutput::Url(_) => ptr::null_mut(),
            })
        }
    }
}

pub struct MuxerBuilder {
    value: Muxer,
}

impl MuxerBuilder {
    pub fn new() -> Self {
        Self {
            value: Muxer {
                ctx: ptr::null_mut(),
                output: MuxerOutput::Url(String::new()),
            },
        }
    }

    unsafe fn init_ctx(
        &mut self,
        dst: Option<&str>,
        format: Option<&str>,
        options: Option<HashMap<String, String>>,
    ) -> Result<()> {
        if !self.value.ctx.is_null() {
            bail!("context already open");
        }

        let ret = avformat_alloc_output_context2(
            &mut self.value.ctx,
            ptr::null_mut(),
            if let Some(format) = format {
                cstr!(format)
            } else {
                ptr::null()
            },
            if let Some(dst) = dst {
                cstr!(dst)
            } else {
                ptr::null()
            },
        );
        bail_ffmpeg!(ret);

        // Setup global header flag
        if (*(*self.value.ctx).oformat).flags & AVFMT_GLOBALHEADER != 0 {
            (*self.value.ctx).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as libc::c_int;
        }

        // Set options on ctx
        if let Some(opts) = options {
            set_opts((*self.value.ctx).priv_data, opts)?;
        }
        Ok(())
    }

    /// Open the muxer with a destination path
    pub unsafe fn with_output_path<'a, T>(
        mut self,
        dst: T,
        format: Option<&'a str>,
        options: Option<HashMap<String, String>>,
    ) -> Result<Self>
    where
        T: Into<&'a str>,
    {
        let path_str = dst.into();
        self.init_ctx(Some(path_str), format, options)?;
        self.value.output = MuxerOutput::Url(path_str.to_string());
        Ok(self)
    }

    /// Create a muxer using a custom IO context
    /// This impl requires [Seek] trait as some muxers need seek support
    pub unsafe fn with_output_write_seek<W>(
        mut self,
        writer: W,
        format: Option<&str>,
        options: Option<HashMap<String, String>>,
    ) -> Result<Self>
    where
        W: WriteSeek + 'static,
    {
        self.init_ctx(None, format, options)?;
        self.value.output = MuxerOutput::WriterSeeker(Some(slimbox_unsize!(writer)));
        Ok(self)
    }

    /// Create a muxer using a custom IO context
    pub unsafe fn with_output_write<W>(
        mut self,
        writer: W,
        format: Option<&str>,
        options: Option<HashMap<String, String>>,
    ) -> Result<Self>
    where
        W: Write + 'static,
    {
        self.init_ctx(None, format, options)?;
        self.value.output = MuxerOutput::Writer(Some(slimbox_unsize!(writer)));
        Ok(self)
    }

    /// Add a stream to the output using an existing encoder
    pub unsafe fn with_stream_encoder(mut self, encoder: &Encoder) -> Result<Self> {
        self.value.add_stream_encoder(encoder)?;
        Ok(self)
    }

    /// Add a stream to the output using an existing input stream (copy)
    pub unsafe fn with_copy_stream(mut self, in_stream: *mut AVStream) -> Result<Self> {
        self.value.add_copy_stream(in_stream)?;
        Ok(self)
    }

    /// Build the muxer
    pub fn build(self) -> Result<Muxer> {
        if self.value.ctx.is_null() {
            bail!("context is null");
        }
        Ok(self.value)
    }
}

impl Muxer {
    pub fn builder() -> MuxerBuilder {
        MuxerBuilder::new()
    }

    /// Add a stream to the output using an existing encoder
    pub unsafe fn add_stream_encoder(&mut self, encoder: &Encoder) -> Result<*mut AVStream> {
        let stream = avformat_new_stream(self.ctx, encoder.codec());
        if stream.is_null() {
            bail!("unable to allocate stream");
        }
        let ret = avcodec_parameters_from_context((*stream).codecpar, encoder.codec_context());
        bail_ffmpeg!(ret);

        // setup other stream params
        let encoder_ctx = encoder.codec_context();
        (*stream).time_base = (*encoder_ctx).time_base;
        (*stream).avg_frame_rate = (*encoder_ctx).framerate;
        (*stream).r_frame_rate = (*encoder_ctx).framerate;

        Ok(stream)
    }

    /// Add a stream to the output using an existing input stream (copy)
    pub unsafe fn add_copy_stream(&mut self, in_stream: *mut AVStream) -> Result<*mut AVStream> {
        let stream = avformat_new_stream(self.ctx, ptr::null_mut());
        if stream.is_null() {
            bail!("unable to allocate stream");
        }

        // copy params from input
        let ret = avcodec_parameters_copy((*stream).codecpar, (*in_stream).codecpar);
        bail_ffmpeg!(ret);

        Ok(stream)
    }

    /// Open the output to start sending packets
    pub unsafe fn open(&mut self) -> Result<()> {
        if (*(*self.ctx).oformat).flags & AVFMT_NOFILE == 0 {
            (*self.ctx).pb = (&mut self.output).try_into()?;
            // if pb is still null, open with ctx.url
            if (*self.ctx).pb.is_null() {
                let ret = avio_open(&mut (*self.ctx).pb, (*self.ctx).url, AVIO_FLAG_WRITE);
                bail_ffmpeg!(ret);
            } else {
                // Don't write buffer, just let the handler functions write directly
                (*self.ctx).flags |= AVIO_FLAG_DIRECT;
            }
        }

        let ret = avformat_write_header(self.ctx, ptr::null_mut());
        bail_ffmpeg!(ret);

        Ok(())
    }

    /// Write a packet to the output
    pub unsafe fn write_packet(&mut self, pkt: *mut AVPacket) -> Result<()> {
        let stream = *(*self.ctx).streams.add((*pkt).stream_index as usize);
        av_packet_rescale_ts(pkt, (*pkt).time_base, (*stream).time_base);
        (*pkt).time_base = (*stream).time_base;

        let ret = av_interleaved_write_frame(self.ctx, pkt);
        bail_ffmpeg!(ret);
        Ok(())
    }

    /// Close the output and write the trailer
    pub unsafe fn close(self) -> Result<()> {
        let ret = av_write_trailer(self.ctx);
        bail_ffmpeg!(ret);
        Ok(())
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                if let MuxerOutput::Writer(_) = self.output {
                    av_free((*(*self.ctx).pb).buffer as *mut _);
                    drop(SlimBox::<dyn Read>::from_raw((*(*self.ctx).pb).opaque));
                }
                avformat_free_context(self.ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_test_frame, Scaler};
    use ffmpeg_sys_the_third::AVCodecID::AV_CODEC_ID_H264;
    use ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
    use ffmpeg_sys_the_third::{AVFrame, AV_PROFILE_H264_MAIN};
    use std::path::PathBuf;

    unsafe fn setup_encoder() -> Result<(*mut AVFrame, Encoder)> {
        std::fs::create_dir_all("test_output")?;
        let path = PathBuf::from("test_output/test.mp4");
        let frame = generate_test_frame();

        // convert frame to YUV
        let mut scaler = Scaler::new();
        let frame = scaler.process_frame(
            frame,
            (*frame).width as u16,
            (*frame).height as u16,
            AV_PIX_FMT_YUV420P,
        )?;

        let encoder = Encoder::new(AV_CODEC_ID_H264)?
            .with_width((*frame).width)
            .with_height((*frame).height)
            .with_pix_fmt(AV_PIX_FMT_YUV420P)
            .with_bitrate(1_000_000)
            .with_framerate(30.0)
            .with_profile(AV_PROFILE_H264_MAIN)
            .with_level(50)
            .open(None)?;
        Ok((frame, encoder))
    }

    unsafe fn write_frames(
        mut muxer: Muxer,
        mut encoder: Encoder,
        frame: *mut AVFrame,
    ) -> Result<()> {
        let mut pts = 0;
        for _z in 0..100 {
            (*frame).pts = pts;
            for pkt in encoder.encode_frame(frame)? {
                muxer.write_packet(pkt)?;
            }
            pts += 1;
        }
        // flush
        for f_pk in encoder.encode_frame(ptr::null_mut())? {
            muxer.write_packet(f_pk)?;
        }
        muxer.close()?;
        Ok(())
    }

    #[test]
    fn encode_mkv() -> Result<()> {
        std::fs::create_dir_all("test_output")?;
        unsafe {
            let path = PathBuf::from("test_output/test_muxer.mp4");
            let (frame, encoder) = setup_encoder()?;

            let mut muxer = Muxer::builder()
                .with_output_path(path.to_str().unwrap(), None, None)?
                .with_stream_encoder(&encoder)?
                .build()?;
            muxer.open()?;
            write_frames(muxer, encoder, frame)?;
        }
        Ok(())
    }

    #[test]
    fn encode_custom_io() -> Result<()> {
        std::fs::create_dir_all("test_output")?;
        unsafe {
            let path = PathBuf::from("test_output/test_custom_muxer.mp4");
            let (frame, encoder) = setup_encoder()?;

            let fout = std::fs::File::create(path)?;
            let mut muxer = Muxer::builder()
                .with_output_write_seek(fout, Some("mp4"), None)?
                .with_stream_encoder(&encoder)?
                .build()?;
            muxer.open()?;
            write_frames(muxer, encoder, frame)?;
        }
        Ok(())
    }

    #[test]
    fn encode_custom_io_non_seek() -> Result<()> {
        std::fs::create_dir_all("test_output")?;
        unsafe {
            let path = PathBuf::from("test_output/test_custom_muxer_no_seek.ts");
            let (frame, encoder) = setup_encoder()?;

            let fout = std::fs::File::create(path)?;
            let mut muxer = Muxer::builder()
                .with_output_write(fout, Some("mpegts"), None)?
                .with_stream_encoder(&encoder)?
                .build()?;
            muxer.open()?;
            write_frames(muxer, encoder, frame)?;
        }
        Ok(())
    }
}