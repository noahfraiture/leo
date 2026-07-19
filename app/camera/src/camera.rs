pub struct Camera {
    pub(crate) status: Status,

    // A camera can have multiple logical channels which can have multiple stream.
    // For simplicity, we assume a camera has a single channel and a single stream.
    // All operations should be done on channel '1'.
    stream: Option<Stream>,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            status: Status::Ready,
            stream: None,
        }
    }

    pub fn pan(&mut self) {}
}

#[derive(Clone, PartialEq)]
pub(crate) enum Status {
    Running,
    Ready,
}

struct Stream {
    video_quality: VideoQuality,
    codec: Codec,
}

enum VideoQuality {}
enum Codec {}
