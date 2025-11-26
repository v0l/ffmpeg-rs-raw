use crate::{AvFrameRef, bail_ffmpeg};
use anyhow::{Result, bail};
use ffmpeg_sys_the_third::{
    AV_NOPTS_VALUE, AVAudioFifo, AVSampleFormat, av_audio_fifo_alloc, av_audio_fifo_read,
    av_audio_fifo_realloc, av_audio_fifo_size, av_audio_fifo_write, av_channel_layout_default,
    av_frame_alloc, av_frame_free, av_frame_get_buffer,
};

pub struct AudioFifo {
    ctx: *mut AVAudioFifo,
    format: AVSampleFormat,
    channels: u16,
    pts: i64,
}

impl AudioFifo {
    pub fn new(format: AVSampleFormat, channels: u16) -> Result<Self> {
        let ctx = unsafe { av_audio_fifo_alloc(format, channels as _, 1) };
        if ctx.is_null() {
            bail!("Could not allocate audio fifo");
        }
        Ok(Self {
            ctx,
            format,
            channels,
            pts: AV_NOPTS_VALUE,
        })
    }

    /// Buffer a frame
    pub fn buffer_frame(&mut self, frame: &AvFrameRef) -> Result<()> {
        unsafe {
            let mut ret =
                av_audio_fifo_realloc(self.ctx, av_audio_fifo_size(self.ctx) + frame.nb_samples);
            bail_ffmpeg!(ret);

            #[cfg(feature = "avutil_version_greater_than_58_22")]
            let buf_ptr = frame.extended_data as *const _;
            #[cfg(not(feature = "avutil_version_greater_than_58_22"))]
            let buf_ptr = frame.extended_data as *mut _;

            ret = av_audio_fifo_write(self.ctx, buf_ptr, frame.nb_samples);
            bail_ffmpeg!(ret);

            // set pts if uninitialized
            if self.pts == AV_NOPTS_VALUE {
                self.pts = frame.pts;
            }
            Ok(())
        }
    }

    /// Get a frame from the buffer if there is enough data
    pub fn get_frame(&mut self, samples_out: usize) -> Result<Option<AvFrameRef>> {
        unsafe {
            if av_audio_fifo_size(self.ctx) >= samples_out as _ {
                let out_frame = av_frame_alloc();
                (*out_frame).nb_samples = samples_out as _;
                (*out_frame).format = self.format as _;
                av_channel_layout_default(&mut (*out_frame).ch_layout, self.channels as _);

                let ret = av_frame_get_buffer(out_frame, 0);
                bail_ffmpeg!(ret, { av_frame_free(&mut (out_frame as *mut _)) });

                #[cfg(feature = "avutil_version_greater_than_58_22")]
                let buf_ptr = (*out_frame).extended_data as *const _;
                #[cfg(not(feature = "avutil_version_greater_than_58_22"))]
                let buf_ptr = (*out_frame).extended_data as *mut _;

                if av_audio_fifo_read(self.ctx, buf_ptr, samples_out as _) < samples_out as _ {
                    av_frame_free(&mut (out_frame as *mut _));
                    bail!("Failed to read audio frame");
                }

                // assign PTS
                (*out_frame).pts = self.pts;
                self.pts += (*out_frame).nb_samples as i64;

                Ok(Some(AvFrameRef::new(out_frame)))
            } else {
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Encoder;
    use ffmpeg_sys_the_third::{AVChannelLayout, av_channel_layout_default};

    #[test]
    fn test_buffer() -> Result<()> {
        unsafe {
            let mut buf = AudioFifo::new(AVSampleFormat::AV_SAMPLE_FMT_FLTP, 2)?;

            let mut enc = Encoder::new_with_name("aac")?
                .with_sample_format(AVSampleFormat::AV_SAMPLE_FMT_FLTP)
                .with_sample_rate(48_000)?
                .with_default_channel_layout(2)
                .open(None)?;

            let demo_frame = av_frame_alloc();
            (*demo_frame).format = AVSampleFormat::AV_SAMPLE_FMT_FLTP as _;
            (*demo_frame).ch_layout = AVChannelLayout::empty();
            av_channel_layout_default(&mut (*demo_frame).ch_layout, 2);
            (*demo_frame).nb_samples = 2048;
            av_frame_get_buffer(demo_frame, 0);
            let demo_frame = AvFrameRef::new(demo_frame);

            let dst_nb_samples = (*enc.codec_context()).frame_size;
            buf.buffer_frame(&demo_frame)?;
            let out_frame = buf.get_frame(dst_nb_samples as usize)?.unwrap();
            for _pkt in enc.encode_frame(Some(&out_frame))? {
                // AvPacketRef will be dropped automatically
            }

            // flush
            for _pkt in enc.encode_frame(None)? {
                // AvPacketRef will be dropped automatically
            }
            Ok(())
        }
    }
}
