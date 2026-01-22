use crate::ffmpeg_sys_the_third::{
    AVFrame, AVPacket, av_frame_clone, av_frame_free, av_packet_clone, av_packet_free,
};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

/// Safe wrapper around AVFrame
pub struct AvFrameRef {
    /// Pointer to the actual frame
    ptr: NonNull<AVFrame>,
    /// Marker for lifetime
    _phantom: PhantomData<AVFrame>,
}

impl Clone for AvFrameRef {
    fn clone(&self) -> Self {
        let clone = unsafe { av_frame_clone(self.ptr.as_ptr()) };
        Self {
            ptr: NonNull::new(clone).unwrap(),
            _phantom: PhantomData::default(),
        }
    }
}

impl Drop for AvFrameRef {
    fn drop(&mut self) {
        unsafe {
            let mut ptr = self.ptr.as_ptr();
            av_frame_free(&mut ptr);
        }
    }
}

impl Deref for AvFrameRef {
    type Target = AVFrame;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl DerefMut for AvFrameRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl AvFrameRef {
    /// Create a new AvFrameRef from a raw AVFrame pointer.
    /// Takes ownership of the frame - caller must not free it.
    pub unsafe fn new(frame: *mut AVFrame) -> Self {
        Self {
            ptr: NonNull::new(frame).expect("Cannot create a frame ref with a null pointer."),
            _phantom: PhantomData::default(),
        }
    }

    pub fn ptr(&self) -> *mut AVFrame {
        self.ptr.as_ptr()
    }
}

unsafe impl Send for AvFrameRef {}

/// Safe wrapper around AVPacket
pub struct AvPacketRef {
    /// Pointer to the actual packet
    ptr: NonNull<AVPacket>,
    /// Marker for lifetime
    _phantom: PhantomData<AVPacket>,
}

impl Clone for AvPacketRef {
    fn clone(&self) -> Self {
        let clone = unsafe { av_packet_clone(self.ptr.as_ptr()) };
        Self {
            ptr: NonNull::new(clone).unwrap(),
            _phantom: PhantomData::default(),
        }
    }
}

impl Drop for AvPacketRef {
    fn drop(&mut self) {
        unsafe {
            let mut ptr = self.ptr.as_ptr();
            av_packet_free(&mut ptr);
        }
    }
}

impl Deref for AvPacketRef {
    type Target = AVPacket;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl DerefMut for AvPacketRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl AvPacketRef {
    /// Create a new AvPacketRef from a raw AVPacket pointer.
    /// Takes ownership of the packet - caller must not free it.
    pub unsafe fn new(packet: *mut AVPacket) -> Self {
        Self {
            ptr: NonNull::new(packet).expect("Cannot create a packet with a null pointer."),
            _phantom: PhantomData::default(),
        }
    }

    pub fn ptr(&self) -> *mut AVPacket {
        self.ptr.as_ptr()
    }
}

unsafe impl Send for AvPacketRef {}

#[cfg(test)]
mod tests {
    use super::*;
    use ffmpeg_sys_the_third::{AVRational, av_frame_alloc, av_frame_get_buffer};
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_send() {
        let frame = unsafe {
            let mut f = AvFrameRef::new(av_frame_alloc());
            f.width = 1280;
            f.height = 720;
            f.format = 0;
            f.pts = 0;
            f.duration = 3000;
            f.time_base = AVRational {
                num: 1,
                den: 90_000,
            };
            av_frame_get_buffer(f.ptr(), 0);
            f
        };

        std::thread::spawn(move || {
            for _ in 0..10 {
                sleep(Duration::from_millis(100));
                assert!(!frame.ptr().is_null());
                assert_eq!(frame.width, 1280);
                assert_eq!(frame.height, 720);
                assert_eq!(frame.format, 0);
                assert_eq!(frame.duration, 3000);
                assert!(!frame.data[0].is_null())
            }
        })
        .join()
        .unwrap();
    }
}
