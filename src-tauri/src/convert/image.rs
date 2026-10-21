use std::io::{Seek, Write};
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, ImageReader, Limits, Rgb, RgbImage};

use super::{ConvertError, Format};

// Decompression-bomb guards: dimension cap plus a hard allocation cap.
const MAX_DIM: u32 = 16_384;
const MAX_ALLOC: u64 = 512 * 1024 * 1024;

pub fn convert(
    src: &Path,
    target: Format,
    jpeg_quality: u8,
    w: &mut (impl Write + Seek),
) -> Result<(), ConvertError> {
    let mut reader = ImageReader::open(src)?.with_guessed_format()?;
    if reader.format().is_none() {
        return Err(ConvertError::Corrupt);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIM);
    limits.max_image_height = Some(MAX_DIM);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let img = reader.decode()?;

    // 16-bit/float/gray+alpha inputs are normalized so every encoder accepts them
    let img = match img {
        DynamicImage::ImageLuma8(_) | DynamicImage::ImageRgb8(_) | DynamicImage::ImageRgba8(_) => {
            img
        }
        _ if img.color().has_alpha() => DynamicImage::ImageRgba8(img.to_rgba8()),
        _ => DynamicImage::ImageRgb8(img.to_rgb8()),
    };

    match target {
        Format::Jpg => {
            let rgb = flatten_on_white(&img);
            JpegEncoder::new_with_quality(w, jpeg_quality.clamp(1, 100)).encode_image(&rgb)?;
        }
        Format::Ico => {
            let img = if img.width() > 256 || img.height() > 256 {
                img.resize(256, 256, image::imageops::FilterType::Lanczos3)
            } else {
                img
            };
            DynamicImage::ImageRgba8(img.to_rgba8()).write_to(w, ImageFormat::Ico)?;
        }
        Format::Png => img.write_to(w, ImageFormat::Png)?,
        Format::Webp => img.write_to(w, ImageFormat::WebP)?, // lossless
        Format::Bmp => img.write_to(w, ImageFormat::Bmp)?,
        Format::Gif => img.write_to(w, ImageFormat::Gif)?,
        Format::Tiff => img.write_to(w, ImageFormat::Tiff)?,
        _ => return Err(ConvertError::Unsupported),
    }
    Ok(())
}

/// JPEG has no alpha: composite over white instead of letting transparency go black.
fn flatten_on_white(img: &DynamicImage) -> RgbImage {
    let rgba = img.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
        let a = u32::from(p[3]);
        let mix = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
        *o = Rgb([mix(p[0]), mix(p[1]), mix(p[2])]);
    }
    out
}
