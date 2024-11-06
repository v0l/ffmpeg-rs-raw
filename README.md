# ffmpeg-rs-raw

**UNSAFE** ffmpeg lib

Providing common utility classes in an unsafe manner.

## Example
Decoding a video file:
```rust
// Demuxer extracts the streams (audio/video) 
// from the container (mp4/mov/avi/mkv etc.)
let mut demuxer = Demuxer::new("./example.mp4");

// Start by "probing" the file to determine stream info etc
let media_info = demuxer.probe_input()?;
println!("{}", media_info);

// Once probing is done, setup the decoder
let mut decoder = Decoder::new();
// enable hardware decoding
decoder.enable_hw_decoder_any();

// Setup decoders for all streams
for ref stream in media_info.channels {
    decoder.setup_decoder(stream, None)?;
}

loop {
    let (mut pkt, stream) = demuxer.get_packet()?;
    let frames = decoder.decode_pkt(pkt, stream)?;
    for (mut frame, _stream) in frames {
        // do something with the frame
        av_frame_free(&mut frame);
    }
    // EOF, its important to do this AFTER [decoder.decode_pkt]
    // the NULL pkt is a flush pkt and will get the last frames from the 
    // decoder (if any)
    if pkt.is_null() {
        break; 
    }
    av_packet_free(&mut pkt);
}
```