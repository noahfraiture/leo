use crate::camera::CameraError;

#[derive(Default)]
pub(crate) struct Camera {
    pub(crate) status: Status,
    pub(crate) pan: f64,

    // A camera can have multiple logical channels with multiple streams.
    // This simulator models one channel with one stream.
    #[allow(dead_code)]
    stream: Option<Stream>,
}

impl Camera {
    pub(crate) fn new() -> Self {
        Self {
            status: Status::Ready,
            stream: None,
            pan: 0.,
        }
    }

    pub(crate) fn validate_channel(channel: u8) -> Result<(), CameraError> {
        if channel != 1 {
            return Err(CameraError::UnsupportedChannel);
        }
        Ok(())
    }

    pub(crate) fn pan(&mut self, channel: u8, offset: f64) -> Result<(), CameraError> {
        Self::validate_channel(channel)?;
        if !(-360.0..=360.0).contains(&offset) {
            return Err(CameraError::PanOutOfRange);
        }
        Ok(())
    }
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
    use super::{Camera, CameraError};

    #[test]
    fn pan_validates_capabilities() {
        assert_eq!(Camera::new().pan(1, 0.0), Ok(()));
        assert_eq!(
            Camera::new().pan(2, 0.0),
            Err(CameraError::UnsupportedChannel)
        );

        for offset in [-361.0, 361.0, f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
            assert_eq!(
                Camera::new().pan(1, offset),
                Err(CameraError::PanOutOfRange)
            );
        }
    }
}
