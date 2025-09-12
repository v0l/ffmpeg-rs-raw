FROM rust:bookworm
WORKDIR /src
RUN apt update && \
    apt install -y \
    build-essential \
    libx264-dev \
    libx265-dev \
    libwebp-dev \
    libvpx-dev \
    libopus-dev \
    libdav1d-dev \
    libvpl-dev \
    libva-dev \
    libva-dev \
    nasm \
    libclang-dev && \
    rm -rf /var/lib/apt/lists/*

## nv-codec-headers
RUN git clone https://git.videolan.org/git/ffmpeg/nv-codec-headers.git && \
    cd nv-codec-headers && \
    make -j$(nproc) install

## FFMPEG
RUN git clone --single-branch --branch release/7.1 https://git.v0l.io/ffmpeg/FFmpeg.git && \
    cd FFmpeg && \
    ./configure \
    --prefix=${FFMPEG_DIR} \
    --disable-programs \
    --disable-doc \
    --disable-network \
    --enable-gpl \
    --enable-libx264 \
    --enable-libx265 \
    --enable-libwebp \
    --enable-libvpx \
    --enable-libopus \
    --enable-libdav1d \
    --enable-libvpl \
    --disable-static \
    --disable-postproc \
    --enable-shared && \
    make -j$(nproc) install
COPY . .
RUN cargo build --release