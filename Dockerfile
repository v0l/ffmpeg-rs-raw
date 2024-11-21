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
    nasm \
    libclang-dev && \
    rm -rf /var/lib/apt/lists/*
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
    --disable-static \
    --disable-postproc \
    --enable-shared && \
    make -j$(nproc) install
COPY . .
RUN cargo build --release