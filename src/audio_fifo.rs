use crate::bail_ffmpeg;
use anyhow::{bail, Result};
use ffmpeg_sys_the_third::{
    av_audio_fifo_alloc, av_audio_fifo_read, av_audio_fifo_realloc, av_audio_fifo_size,
    av_audio_fifo_write, av_channel_layout_copy, av_frame_alloc, av_frame_free,
    av_frame_get_buffer, AVAudioFifo, AVFrame, AVSampleFormat,
};

pub struct AudioFifo {
    ctx: *mut AVAudioFifo,
    pts: i64,
}

impl AudioFifo {
    pub fn new(format: AVSampleFormat, channels: u16) -> Result<Self> {
        let ctx = unsafe { av_audio_fifo_alloc(format, channels as _, 1) };
        if ctx.is_null() {
            bail!("Could not allocate audio fifo");
        }
        Ok(Self { ctx, pts: 0 })
    }

    /// Buffer a resampled frame, and get a frame from the buffer with the desired size
    pub unsafe fn buffer_frame(
        &mut self,
        frame: *mut AVFrame,
        samples_out: usize,
    ) -> Result<Option<*mut AVFrame>> {
        let mut ret =
            av_audio_fifo_realloc(self.ctx, av_audio_fifo_size(self.ctx) + (*frame).nb_samples);
        bail_ffmpeg!(ret);

        #[cfg(feature = "avutil_version_greater_than_58_22")]
        let buf_ptr = (*frame).extended_data as *const _;
        #[cfg(not(feature = "avutil_version_greater_than_58_22"))]
        let buf_ptr = (*frame).extended_data as *mut _;

        ret = av_audio_fifo_write(self.ctx, buf_ptr, (*frame).nb_samples);
        bail_ffmpeg!(ret);

        if av_audio_fifo_size(self.ctx) >= samples_out as _ {
            let mut out_frame = av_frame_alloc();
            (*out_frame).nb_samples = samples_out as _;
            (*out_frame).format = (*frame).format as _;
            ret = av_channel_layout_copy(&mut (*out_frame).ch_layout, &(*frame).ch_layout);
            bail_ffmpeg!(ret, { av_frame_free(&mut out_frame) });

            ret = av_frame_get_buffer(out_frame, 0);
            bail_ffmpeg!(ret, { av_frame_free(&mut out_frame) });

            #[cfg(feature = "avutil_version_greater_than_58_22")]
            let buf_ptr = (*out_frame).extended_data as *const _;
            #[cfg(not(feature = "avutil_version_greater_than_58_22"))]
            let buf_ptr = (*out_frame).extended_data as *mut _;

            if av_audio_fifo_read(self.ctx, buf_ptr, samples_out as _) < samples_out as _ {
                av_frame_free(&mut out_frame);
                bail!("Failed to read audio frame");
            }

            // assign PTS
            (*out_frame).pts = self.pts;
            self.pts += (*out_frame).nb_samples as i64;

            Ok(Some(out_frame))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Encoder;
    use ffmpeg_sys_the_third::{av_channel_layout_default, av_packet_free, AVChannelLayout};
    use std::ptr;
    #[test]
    fn test_buffer() -> Result<()> {
        unsafe {
            let mut buf = AudioFifo::new(AVSampleFormat::AV_SAMPLE_FMT_S16, 2)?;

            let mut enc = Encoder::new_with_name("libfdk_aac")?
                .with_sample_format(AVSampleFormat::AV_SAMPLE_FMT_S16)
                .with_sample_rate(48_000)?
                .with_default_channel_layout(2)
                .open(None)?;

            let mut demo_frame = av_frame_alloc();
            (*demo_frame).format = AVSampleFormat::AV_SAMPLE_FMT_S16 as _;
            (*demo_frame).ch_layout = AVChannelLayout::empty();
            av_channel_layout_default(&mut (*demo_frame).ch_layout, 2);
            (*demo_frame).nb_samples = 2048;
            av_frame_get_buffer(demo_frame, 0);

            let dst_nb_samples = (*enc.codec_context()).frame_size;
            let mut out_frame = buf
                .buffer_frame(demo_frame, dst_nb_samples as usize)?
                .unwrap();
            for mut pkt in enc.encode_frame(out_frame)? {
                av_packet_free(&mut pkt);
            }

            av_frame_free(&mut demo_frame);
            av_frame_free(&mut out_frame);

            // flush
            for mut pkt in enc.encode_frame(ptr::null_mut())? {
                av_packet_free(&mut pkt);
            }
            Ok(())
        }
    }
}
