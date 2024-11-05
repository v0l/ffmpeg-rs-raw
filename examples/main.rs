use ffmpeg_rs_raw::{Decoder, Demuxer, DemuxerInfo};
use ffmpeg_sys_the_third::AVHWDeviceType::{AV_HWDEVICE_TYPE_CUDA, AV_HWDEVICE_TYPE_VDPAU};
use ffmpeg_sys_the_third::{av_frame_free, av_packet_free};
use std::env::args;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;

fn main() {
    let name = args().next().unwrap_or("main".to_string());
    let path = if let Some(path) = args().skip(1).next() {
        PathBuf::from(path)
    } else {
        eprintln!("Usage: {} <path>", name);
        std::process::exit(1);
    };

    let cd = read_as_custom_io(path.clone());
    scan_input(cd);

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
        println!("{}", info);
        decode_input(demuxer, info);
    }
}

unsafe fn decode_input(demuxer: Demuxer, info: DemuxerInfo) {
    let mut decoder = Decoder::new();
    decoder.enable_hw_decoder(AV_HWDEVICE_TYPE_VDPAU);
    decoder.enable_hw_decoder(AV_HWDEVICE_TYPE_CUDA);
    for ref stream in info.channels {
        decoder
            .setup_decoder(stream, None)
            .expect("decoder setup failed");
    }
    println!("{}", decoder);
    loop_decoder(demuxer, decoder);
}

unsafe fn loop_decoder(mut demuxer: Demuxer, mut decoder: Decoder) {
    loop {
        let (mut pkt, stream) = demuxer.get_packet().expect("demuxer failed");
        if pkt.is_null() {
            break; // EOF
        }
        if let Ok(frames) = decoder.decode_pkt(pkt, stream) {
            for (mut frame, _stream) in frames {
                // do nothing but decode entire stream
                av_frame_free(&mut frame);
            }
        }
        av_packet_free(&mut pkt);
    }
}
