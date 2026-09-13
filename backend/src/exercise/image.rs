use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, metadata::Orientation};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Error, ImageAsset, ImageMediaType, Result};

/// Compressed input limit. Imports read one bounded file at a time.
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// Decode limit checked from dimensions before allocating the full image.
pub const MAX_IMAGE_PIXELS: u64 = 32_000_000;

/// Orientation-correct PNG derivative for thumbnails or model inputs, separate from originals.
#[derive(Clone, Debug)]
pub struct ReferencePreview {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn inspect(bytes: &[u8]) -> Result<ImageAsset> {
    let (image, media_type, orientation) = decode(bytes)?;
    Ok(ImageAsset {
        id: Uuid::new_v4(),
        sha256: hash(bytes),
        byte_length: bytes.len() as u64,
        media_type,
        width: image.width(),
        height: image.height(),
        orientation: orientation.to_exif(),
    })
}

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Decode original bytes from a library or run snapshot into an orientation-correct PNG derivative.
pub fn preview_reference_bytes(bytes: &[u8], maximum_long_edge: u32) -> Result<ReferencePreview> {
    if maximum_long_edge == 0 {
        return Err(Error::Invalid(
            "reference preview size must be positive".into(),
        ));
    }
    let (mut image, _, orientation) = decode(bytes)?;
    image.apply_orientation(orientation);
    if image.width().max(image.height()) > maximum_long_edge {
        image = image.thumbnail(maximum_long_edge, maximum_long_edge);
    }
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    Ok(ReferencePreview {
        png: png.into_inner(),
        width: image.width(),
        height: image.height(),
    })
}

fn decode(bytes: &[u8]) -> Result<(DynamicImage, ImageMediaType, Orientation)> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(Error::ImageTooLarge);
    }
    let format = image::guess_format(bytes).map_err(|_| Error::UnsupportedImage)?;
    let media_type = match format {
        ImageFormat::Jpeg => ImageMediaType::Jpeg,
        ImageFormat::Png => ImageMediaType::Png,
        _ => return Err(Error::UnsupportedImage),
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(Error::ImageTooLarge);
    }
    let orientation = decoder.orientation()?;
    // Decoding the complete pixel data rejects files whose headers alone look valid.
    let image = DynamicImage::from_decoder(decoder)?;
    Ok((image, media_type, orientation))
}
