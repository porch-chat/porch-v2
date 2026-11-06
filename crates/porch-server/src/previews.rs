//! Decode only bounded raster images off the reactor, never original SVG/HTML.
use image::{ImageFormat, ImageReader, Limits};
use std::io::{self, Cursor, Read};
pub const MAX_PREVIEW_SOURCE: u64 = 16 * 1024 * 1024;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "image preview unavailable")
}
/// Caller holds a bounded blocking-work permit. Memory limits are best effort;
/// strict 4096px dimensions and a 16MiB encoded input bound are also enforced.
pub fn thumbnail(mut source: impl Read) -> io::Result<Vec<u8>> {
    let mut input = Vec::new();
    source
        .by_ref()
        .take(MAX_PREVIEW_SOURCE + 1)
        .read_to_end(&mut input)?;
    if input.len() as u64 > MAX_PREVIEW_SOURCE {
        return Err(invalid());
    }
    let mut reader = ImageReader::new(Cursor::new(input)).with_guessed_format()?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    ) {
        return Err(invalid());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| invalid())?.thumbnail(480, 320);
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| invalid())?;
    Ok(output.into_inner())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_reencode_and_reject_active_content() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1000, 600)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let preview = thumbnail(Cursor::new(png.into_inner())).unwrap();
        let decoded = image::load_from_memory(&preview).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (480, 288));
        for bad in [
            b"<svg onload=alert(1)>".as_slice(),
            b"<html>bad</html>",
            b"\x89PNG\r\n\x1a\ntruncated",
        ] {
            assert!(thumbnail(Cursor::new(bad)).is_err());
        }
    }
    #[test]
    fn excessive_dimensions_are_rejected() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(4097, 1)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        assert!(thumbnail(Cursor::new(png.into_inner())).is_err());
    }
}
