use ffmpeg_rs_raw::{Decoder, Demuxer, DemuxerInfo, Scaler, get_frame_from_hw};
use ffmpeg_sys_the_third::{AVMediaType, AVPixelFormat};
use log::{error, info};
use std::env::args;
use std::path::PathBuf;

fn main() {
    env_logger::init();
    let name = args().next().unwrap_or("main".to_string());
    let path = if let Some(path) = args().nth(1) {
        PathBuf::from(path)
    } else {
        error!("Usage: {} <path>", name);
        std::process::exit(1);
    };

    let fd = read_as_file(path.clone());
    scan_input(fd);
}

fn read_as_file(path_buf: PathBuf) -> Demuxer {
    Demuxer::new(path_buf.to_str().unwrap()).unwrap()
}

fn scan_input(mut demuxer: Demuxer) {
    let info = unsafe { demuxer.probe_input().expect("demuxer failed") };
    info!("{:?}", info);
    decode_input(demuxer, info);
}

fn decode_input(demuxer: Demuxer, info: DemuxerInfo) {
    let mut decoder = Decoder::new();
    decoder.enable_hw_decoder_any();

    for ref stream in info.streams {
        decoder
            .setup_decoder(stream, None)
            .expect("decoder setup failed");
    }
    loop_decoder(demuxer, decoder);
}

fn loop_decoder(mut demuxer: Demuxer, mut decoder: Decoder) {
    let mut scale = Scaler::new();
    loop {
        let (pkt, stream) = unsafe { demuxer.get_packet().expect("demuxer failed") };
        let Some(pkt) = pkt else {
            break; // EOF
        };
        let media_type = unsafe { (*(*stream).codecpar).codec_type };
        // only decode audio/video
        if media_type != AVMediaType::AVMEDIA_TYPE_VIDEO
            && media_type != AVMediaType::AVMEDIA_TYPE_AUDIO
        {
            continue;
        }
        if let Ok(frames) = decoder.decode_pkt(Some(&pkt)) {
            for (frame, _stream) in frames {
                // do nothing but decode entire stream
                if media_type == AVMediaType::AVMEDIA_TYPE_VIDEO {
                    let frame = get_frame_from_hw(frame).expect("get frame failed");
                    let _new_frame = scale
                        .process_frame(&frame, 512, 512, AVPixelFormat::AV_PIX_FMT_RGBA)
                        .expect("scale failed");
                }
            }
        }
    }
}
