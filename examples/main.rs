use ffmpeg_rs_raw::{Decoder, Demuxer, DemuxerInfo, Filter};
use ffmpeg_sys_the_third::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA;
use ffmpeg_sys_the_third::{av_frame_free, av_packet_free, AVMediaType};
use log::{error, info};
use std::env::args;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;

fn main() {
    env_logger::init();
    let name = args().next().unwrap_or("main".to_string());
    let path = if let Some(path) = args().skip(1).next() {
        PathBuf::from(path)
    } else {
        error!("Usage: {} <path>", name);
        std::process::exit(1);
    };

    let fd = read_as_file(path.clone());
    scan_input(fd);
}

fn read_as_custom_io(path: PathBuf) -> Demuxer {
    let mut data: Vec<u8> = Vec::new();
    File::open(path).unwrap().read_to_end(&mut data).unwrap();
    let reader = Cursor::new(data);
    Demuxer::new_custom_io(reader, None)
}

fn read_as_file(path_buf: PathBuf) -> Demuxer {
    Demuxer::new(path_buf.to_str().unwrap())
}

fn scan_input(mut demuxer: Demuxer) {
    unsafe {
        let info = demuxer.probe_input().expect("demuxer failed");
        info!("{}", info);
        decode_input(demuxer, info);
    }
}

unsafe fn decode_input(demuxer: Demuxer, info: DemuxerInfo) {
    let mut decoder = Decoder::new();
    //decoder.enable_hw_decoder_any();
    decoder.enable_hw_decoder(AV_HWDEVICE_TYPE_CUDA);
    for ref stream in info.channels {
        decoder
            .setup_decoder(stream, None)
            .expect("decoder setup failed");
    }
    info!("{}", decoder);
    loop_decoder(demuxer, decoder);
}

unsafe fn loop_decoder(mut demuxer: Demuxer, mut decoder: Decoder) {
    let mut filter =
        Filter::parse(&format!("scale_cuda=w={}:h={}", -2, 1080)).expect("filter add failed");

    loop {
        let (mut pkt, stream) = demuxer.get_packet().expect("demuxer failed");
        if pkt.is_null() {
            break; // EOF
        }
        let media_type = (*(*stream).codecpar).codec_type;
        // only decode audio/video
        if media_type != AVMediaType::AVMEDIA_TYPE_VIDEO
            && media_type != AVMediaType::AVMEDIA_TYPE_AUDIO
        {
            av_packet_free(&mut pkt);
            continue;
        }
        if let Ok(frames) = decoder.decode_pkt(pkt, stream) {
            for (mut frame, _stream) in frames {
                // do nothing but decode entire stream
                if media_type == AVMediaType::AVMEDIA_TYPE_VIDEO {
                    let mut new_frame = filter.process_frame(frame).expect("scale failed");
                    av_frame_free(&mut new_frame);
                }
                av_frame_free(&mut frame);
            }
        }
        av_packet_free(&mut pkt);
    }
}
