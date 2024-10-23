use ffmpeg_rs_raw::{Decoder, Demuxer};
use ffmpeg_sys_the_third::{av_frame_free, av_packet_free};
use std::env::args;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

struct DropTest {
    inner: File,
}

impl Read for DropTest {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Drop for DropTest {
    fn drop(&mut self) {
        println!("Dropped!");
    }
}

fn main() {
    let name = args().next().unwrap_or("main".to_string());
    let path = if let Some(path) = args().skip(1).next() {
        PathBuf::from(path)
    } else {
        eprintln!("Usage: {} <path>", name);
        std::process::exit(1);
    };

    let file = File::open(path).unwrap();
    let mut demuxer = Demuxer::new_custom_io(DropTest { inner: file });
    unsafe {
        let info = demuxer.probe_input().expect("demuxer failed");
        println!("{}", info);

        let mut decoder = Decoder::new();
        loop {
            let (mut pkt, stream) = demuxer.get_packet().expect("demuxer failed");
            if pkt.is_null() {
                break; // EOF
            }
            if let Ok(frames) = decoder.decode_pkt(pkt, stream) {
                for (mut frame, stream) in frames {
                    // do nothing but decode entire stream
                    av_frame_free(&mut frame);
                }
            }
            av_packet_free(&mut pkt);
        }
    }
}
