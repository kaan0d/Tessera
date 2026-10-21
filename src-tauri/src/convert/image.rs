//! Still-image pipeline: decode (raster, SVG, PDF page, RAW preview) -> orientation ->
//! resize -> encode, optionally searching for a target file size. Decoders all run under
//! the dimension and allocation caps; output carries no EXIF/GPS/XMP (the encoders write
//! none), apart from the ICC profile that `strip_metadata = false` keeps in PDF-embedded JPEGs.

use std::io::{Cursor, Seek, Write};
use std::path::Path;

use image::codecs::avif::AvifEncoder;
use image::codecs::jpeg::{JpegDecoder, JpegEncoder};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, Rgb, RgbImage};

use super::transform::{self, MAX_ALLOC, MAX_DIM};
use super::{pdf, raw, size, svg, ConvertError, Format, Options};

const AVIF_SPEED: u8 = 8;
const PASSTHROUGH_MAX: u64 = 100 * 1024 * 1024;

/// Converts the still image (or PDF page) at `src`, whose format is `from`, to `target`.
/// `photo` says the content is a photograph, which decides lossy (JPEG) or lossless
/// (Flate) storage inside a PDF.
pub fn convert(
    src: &Path,
    from: Format,
    target: Format,
    photo: bool,
    opts: &Options,
    w: &mut (impl Write + Seek),
) -> Result<(), ConvertError> {
    if let Some(done) = jpeg_into_pdf(src, from, target, opts, w) {
        return done;
    }
    let img = load(src, from, opts)?;
    let q = opts.quality.jpeg;
    match opts.max_bytes {
        None => encode(&img, target, q, photo, w),
        Some(max) => {
            let lossy =
                matches!(target, Format::Jpg | Format::Avif) || (target == Format::Pdf && photo);
            // ponytail: each attempt is held in memory; uncompressed targets (BMP) of huge
            // images peak around twice the pixel buffer. Cap by MAX_ALLOC upstream.
            let bytes = size::fit(&img, max, lossy, q, |i, q| {
                let mut c = Cursor::new(Vec::new());
                encode(i, target, q, photo, &mut c)?;
                Ok(c.into_inner())
            })?;
            w.write_all(&bytes)?;
            Ok(())
        }
    }
}

/// Decodes `src` into pixels: oriented (if asked) and resized (if asked), normalized to
/// 8-bit gray/RGB/RGBA.
pub fn load(src: &Path, from: Format, opts: &Options) -> Result<DynamicImage, ConvertError> {
    let (img, exif) = match from {
        Format::Svg => return svg::render(src, opts.resize),
        Format::Pdf => return pdf::render_page(src, opts.page, opts.resize),
        Format::Raw => raw::decode(src)?,
        _ => {
            let o = matches!(
                from,
                Format::Jpg | Format::Webp | Format::Png | Format::Tiff
            )
            .then(|| transform::file_orientation(src))
            .flatten();
            (decode_raster(src)?, o)
        }
    };
    let mut img = img;
    if opts.preserve_orientation {
        img = transform::orient(img, exif);
    }
    if let Some(r) = opts.resize {
        img = transform::apply(img, r)?;
    }
    // 16-bit/float/gray+alpha inputs are normalized so every encoder accepts them
    Ok(match img {
        DynamicImage::ImageLuma8(_) | DynamicImage::ImageRgb8(_) | DynamicImage::ImageRgba8(_) => {
            img
        }
        _ if img.color().has_alpha() => DynamicImage::ImageRgba8(img.to_rgba8()),
        _ => DynamicImage::ImageRgb8(img.to_rgb8()),
    })
}

fn decode_raster(src: &Path) -> Result<DynamicImage, ConvertError> {
    let mut reader = ImageReader::open(src)?.with_guessed_format()?;
    if reader.format().is_none() {
        return Err(ConvertError::Corrupt);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIM);
    limits.max_image_height = Some(MAX_DIM);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// Encodes `img` as `target`; `q` is the JPEG/AVIF quality.
pub fn encode(
    img: &DynamicImage,
    target: Format,
    q: u8,
    photo: bool,
    w: &mut (impl Write + Seek),
) -> Result<(), ConvertError> {
    let q = q.clamp(1, 100);
    match target {
        Format::Jpg => {
            JpegEncoder::new_with_quality(w, q).encode_image(&flatten_on_white(img))?;
        }
        Format::Avif => {
            img.write_with_encoder(AvifEncoder::new_with_speed_quality(w, AVIF_SPEED, q))?
        }
        Format::Ico => {
            let img = if img.width() > 256 || img.height() > 256 {
                img.resize(256, 256, image::imageops::FilterType::Lanczos3)
            } else {
                img.clone()
            };
            DynamicImage::ImageRgba8(img.to_rgba8()).write_to(w, ImageFormat::Ico)?;
        }
        Format::Png => img.write_to(w, ImageFormat::Png)?,
        Format::Webp => img.write_to(w, ImageFormat::WebP)?, // lossless
        Format::Bmp => img.write_to(w, ImageFormat::Bmp)?,
        Format::Gif => img.write_to(w, ImageFormat::Gif)?,
        Format::Tiff => img.write_to(w, ImageFormat::Tiff)?,
        Format::Pdf if photo => {
            let gray = !img.color().has_color();
            let mut jpg = Vec::new();
            if gray {
                JpegEncoder::new_with_quality(&mut jpg, q).encode_image(&img.to_luma8())?;
            } else {
                JpegEncoder::new_with_quality(&mut jpg, q).encode_image(&flatten_on_white(img))?;
            }
            pdf::write(
                &pdf::Payload::Jpeg {
                    data: &jpg,
                    w: img.width(),
                    h: img.height(),
                    gray,
                },
                w,
            )?;
        }
        Format::Pdf => pdf::write(&pdf::Payload::Pixels(img), w)?,
        _ => return Err(ConvertError::Unsupported),
    }
    Ok(())
}

/// JPEG -> PDF with no pixel change wanted: the original JPEG data is embedded as is
/// (minus metadata), so there is no second lossy pass. None when that does not apply.
fn jpeg_into_pdf(
    src: &Path,
    from: Format,
    target: Format,
    opts: &Options,
    w: &mut impl Write,
) -> Option<Result<(), ConvertError>> {
    if from != Format::Jpg
        || target != Format::Pdf
        || opts.resize.is_some()
        || opts.max_bytes.is_some()
    {
        return None;
    }
    if opts.preserve_orientation && transform::file_orientation(src).is_some_and(|o| o != 1) {
        return None; // PDF viewers ignore EXIF orientation, so the pixels must be turned
    }
    if std::fs::metadata(src).ok()?.len() > PASSTHROUGH_MAX {
        return None;
    }
    let data = pdf::clean_jpeg(&std::fs::read(src).ok()?, !opts.strip_metadata)?;
    let dec = JpegDecoder::new(Cursor::new(&data)).ok()?;
    let (iw, ih) = dec.dimensions();
    let gray = match dec.color_type() {
        image::ColorType::L8 => true,
        image::ColorType::Rgb8 => false,
        _ => return None,
    };
    transform::check_size(iw, ih).ok()?;
    Some(pdf::write(
        &pdf::Payload::Jpeg {
            data: &data,
            w: iw,
            h: ih,
            gray,
        },
        w,
    ))
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
