use std::env;

fn main() {
    // re-export ffmpeg-sys-the-third features
    // https://github.com/FFmpeg/FFmpeg/blob/master/doc/APIchanges
    for (name, _value) in env::vars() {
        if name.starts_with("DEP_FFMPEG_") && !name.starts_with("DEP_FFMPEG_CHECK_") {
            let feature_name = name["DEP_FFMPEG_".len()..name.len()].to_lowercase();
            println!(r#"cargo::rustc-check-cfg=cfg(feature, values("{feature_name}"))"#);
            println!(r#"cargo::rustc-cfg=feature="{feature_name}""#);
        }
    }
}
