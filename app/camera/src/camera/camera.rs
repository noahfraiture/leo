use crate::camera::Error;

#[derive(Default)]
pub(crate) struct Camera;

impl Camera {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn validate_channel(channel: u8) -> Result<(), Error> {
        if channel != 1 {
            return Err(Error::UnsupportedChannel);
        }
        Ok(())
    }

    pub(crate) fn pan(&mut self, channel: u8, offset: f64) -> Result<(), Error> {
        Self::validate_channel(channel)?;
        if !(-360.0..=360.0).contains(&offset) {
            return Err(Error::PanOutOfRange);
        }
        Ok(())
    }

    pub(crate) fn tilt(&mut self, channel: u8, offset: f64) -> Result<(), Error> {
        Self::validate_channel(channel)?;
        if !(-360.0..=360.0).contains(&offset) {
            return Err(Error::PanOutOfRange);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Camera, Error};

    #[test]
    fn pan_validates_capabilities() {
        assert_eq!(Camera::new().pan(1, 0.0), Ok(()));
        assert_eq!(Camera::new().pan(2, 0.0), Err(Error::UnsupportedChannel));

        for offset in [-361.0, 361.0, f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
            assert_eq!(Camera::new().pan(1, offset), Err(Error::PanOutOfRange));
        }
    }
}
