#[derive(Default)]
pub struct Camera {
    pub(crate) status: Status,

    // A camera can have multiple logical channels with multiple streams.
    // This simulator models one channel with one stream.
    #[allow(dead_code)]
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

#[derive(Clone, Default, PartialEq)]
pub(crate) enum Status {
    Running,
    #[default]
    Ready,
}

#[allow(dead_code)]
struct Stream {
    video_quality: VideoQuality,
    codec: Codec,
}

enum VideoQuality {}
enum Codec {}

#[cfg(test)]
mod tests {
    use super::Camera;

    #[test]
    fn new_camera_has_no_active_stream() {
        assert!(Camera::new().stream.is_none());
    }
}
