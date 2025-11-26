use crate::ffmpeg_sys_the_third::{
    AVFrame, AVPacket, av_frame_clone, av_frame_free, av_packet_clone, av_packet_free,
};
use std::ops::Deref;

/// Safe wrapper around AVFrame
pub struct AvFrameRef {
    frame: *mut AVFrame,
}

impl Clone for AvFrameRef {
    fn clone(&self) -> Self {
        let clone = unsafe { av_frame_clone(self.frame) };
        Self { frame: clone }
    }
}

impl Drop for AvFrameRef {
    fn drop(&mut self) {
        unsafe {
            av_frame_free(&mut self.frame);
        }
        self.frame = std::ptr::null_mut();
    }
}

impl Deref for AvFrameRef {
    type Target = AVFrame;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.frame }
    }
}

impl AvFrameRef {
    /// Create a new AvFrameRef from a raw AVFrame pointer.
    /// Takes ownership of the frame - caller must not free it.
    pub unsafe fn new(frame: *mut AVFrame) -> Self {
        Self { frame }
    }

    pub fn ptr(&self) -> *mut AVFrame {
        self.frame
    }
}

/// Safe wrapper around AVPacket
pub struct AvPacketRef {
    packet: *mut AVPacket,
}

impl Clone for AvPacketRef {
    fn clone(&self) -> Self {
        let clone = unsafe { av_packet_clone(self.packet) };
        Self { packet: clone }
    }
}

impl Drop for AvPacketRef {
    fn drop(&mut self) {
        unsafe {
            av_packet_free(&mut self.packet);
        }
        self.packet = std::ptr::null_mut();
    }
}

impl Deref for AvPacketRef {
    type Target = AVPacket;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.packet }
    }
}

impl AvPacketRef {
    /// Create a new AvPacketRef from a raw AVPacket pointer.
    /// Takes ownership of the packet - caller must not free it.
    pub unsafe fn new(packet: *mut AVPacket) -> Self {
        Self { packet }
    }

    pub fn ptr(&self) -> *mut AVPacket {
        self.packet
    }
}
