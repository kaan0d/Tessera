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
    // a JPEG that only has to lose its metadata keeps its pixels: no second lossy pass, no growth
    if from == Format::Jpg && target == Format::Jpg && opts.strip_metadata {
        let upright = transform::file_orientation(src).is_none_or(|o| o == 1);
        if let Some((data, ..)) = upright.then(|| jpeg_passthrough(src, opts)).flatten() {
            w.write_all(&data)?;
            return Ok(());
        }
    }
    opts.check_cancel()?;
    let img = load(src, from, opts)?;
    opts.check_cancel()?;
    let lossy_webp = target == Format::Webp && opts.lossy_webp().is_some();
    let q = if target == Format::Webp {
        opts.quality.webp
    } else {
        opts.quality.jpeg
    };
    match opts.max_bytes {
        None => encode_job(&img, target, q, photo, opts, w),
        Some(max) => {
            let lossy = matches!(target, Format::Jpg | Format::Avif)
                || (target == Format::Pdf && photo)
                || lossy_webp;
            // ponytail: each attempt is held in memory; uncompressed targets (BMP) of huge
            // images peak around twice the pixel buffer. Cap by MAX_ALLOC upstream.
            let bytes = size::fit(&img, max, lossy, q, |i, q| {
                opts.check_cancel()?;
                let mut c = Cursor::new(Vec::new());
                encode_job(i, target, q, photo, opts, &mut c)?;
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

/// `encode`, except lossy WebP goes through ffmpeg's libwebp when the job has one. If that
/// fails (not for a cancel) the pure-Rust lossless encoder writes the file instead.
fn encode_job(
    img: &DynamicImage,
    target: Format,
    q: u8,
    photo: bool,
    opts: &Options,
    w: &mut (impl Write + Seek),
) -> Result<(), ConvertError> {
    if let (Format::Webp, Some(ff)) = (target, opts.lossy_webp()) {
        match super::ffmpeg::encode_webp(ff, img, q, opts.cancel.as_deref()) {
            Ok(bytes) => return Ok(w.write_all(&bytes)?),
            Err(ConvertError::Cancelled) => return Err(ConvertError::Cancelled),
            Err(_) => {}
        }
    }
    encode(img, target, q, photo, w)
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
        Format::Ico => write_ico(img, w)?,
        Format::Png => img.write_to(w, ImageFormat::Png)?,
        Format::Webp => img.write_to(w, ImageFormat::WebP)?, // lossless
        Format::Bmp => img.write_to(w, ImageFormat::Bmp)?,
        Format::Gif => img.write_to(w, ImageFormat::Gif)?,
        Format::Tiff => img.write_to(w, ImageFormat::Tiff)?,
        Format::Pdf if photo => {
            let (data, gray) = photo_jpeg(img, q)?;
            pdf::write(
                &pdf::Payload::Jpeg {
                    data: &data,
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

/// A favicon-ready icon: 16, 32, 48 and 256 px entries, each the image scaled to fit a square
/// of that size and centred on transparency.
fn write_ico(img: &DynamicImage, w: impl Write) -> Result<(), ConvertError> {
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    use image::imageops::{overlay, FilterType};
    use image::{ExtendedColorType, RgbaImage};
    let frames = [16u32, 32, 48, 256]
        .iter()
        .map(|&s| {
            let fit = img.resize(s, s, FilterType::Lanczos3).to_rgba8();
            let mut square = RgbaImage::new(s, s);
            overlay(
                &mut square,
                &fit,
                i64::from((s - fit.width()) / 2),
                i64::from((s - fit.height()) / 2),
            );
            IcoFrame::as_png(square.as_raw(), s, s, ExtendedColorType::Rgba8)
        })
        .collect::<Result<Vec<_>, _>>()?;
    IcoEncoder::new(w).encode_images(&frames)?;
    Ok(())
}

fn photo_jpeg(img: &DynamicImage, q: u8) -> Result<(Vec<u8>, bool), ConvertError> {
    let gray = !img.color().has_color();
    let mut jpg = Vec::new();
    if gray {
        JpegEncoder::new_with_quality(&mut jpg, q).encode_image(&img.to_luma8())?;
    } else {
        JpegEncoder::new_with_quality(&mut jpg, q).encode_image(&flatten_on_white(img))?;
    }
    Ok((jpg, gray))
}

/// One PDF page worth of image data, owned.
pub enum Page {
    Jpeg {
        data: Vec<u8>,
        w: u32,
        h: u32,
        gray: bool,
    },
    Pixels(DynamicImage),
}

impl Page {
    pub fn payload(&self) -> pdf::Payload<'_> {
        match self {
            Page::Jpeg { data, w, h, gray } => pdf::Payload::Jpeg {
                data,
                w: *w,
                h: *h,
                gray: *gray,
            },
            Page::Pixels(i) => pdf::Payload::Pixels(i),
        }
    }

    /// Approximate memory held.
    pub fn bytes(&self) -> u64 {
        match self {
            Page::Jpeg { data, .. } => data.len() as u64,
            Page::Pixels(i) => u64::from(i.width()) * u64::from(i.height()) * 4,
        }
    }
}

/// The page for one image of a merged PDF: same rules as a single image -> PDF job.
pub fn pdf_page(src: &Path, from: Format, opts: &Options) -> Result<Page, ConvertError> {
    pdf_page_as(src, from, opts, false)
}

/// Same, but every page is stored as a JPEG (alpha flattened on white): the way to meet a size limit.
pub fn pdf_page_lossy(src: &Path, from: Format, opts: &Options) -> Result<Page, ConvertError> {
    pdf_page_as(src, from, opts, true)
}

fn pdf_page_as(
    src: &Path,
    from: Format,
    opts: &Options,
    lossy: bool,
) -> Result<Page, ConvertError> {
    if from == Format::Jpg {
        if let Some((data, w, h, gray)) = jpeg_passthrough(src, opts) {
            return Ok(Page::Jpeg { data, w, h, gray });
        }
    }
    let mut img = load(src, from, opts)?;
    if lossy && img.color().has_alpha() {
        img = DynamicImage::ImageRgb8(flatten_on_white(&img));
    }
    if lossy || matches!(from, Format::Jpg | Format::Raw) {
        let (data, gray) = photo_jpeg(&img, opts.quality.jpeg.clamp(1, 100))?;
        return Ok(Page::Jpeg {
            data,
            w: img.width(),
            h: img.height(),
            gray,
        });
    }
    Ok(Page::Pixels(img))
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
    if from != Format::Jpg || target != Format::Pdf {
        return None;
    }
    let (data, iw, ih, gray) = jpeg_passthrough(src, opts)?;
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

fn jpeg_passthrough(src: &Path, opts: &Options) -> Option<(Vec<u8>, u32, u32, bool)> {
    if opts.resize.is_some() || opts.max_bytes.is_some() {
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
    Some((data, iw, ih, gray))
}

/// JPEG has no alpha: composite over white instead of letting transparency go black.
fn flatten_on_white(img: &DynamicImage) -> RgbImage {
    let rgba = img.to_rgba8();
    if rgba.pixels().any(|p| p[3] < 255) {
        super::ALPHA_FLATTENED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
        let a = u32::from(p[3]);
        let mix = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
        *o = Rgb([mix(p[0]), mix(p[1]), mix(p[2])]);
    }
    out
}

#[cfg(test)]
mod ico_tests {
    use super::*;

    #[test]
    fn icons_carry_the_four_favicon_sizes() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::new(40, 20));
        let mut out = Vec::new();
        write_ico(&img, &mut out).unwrap();
        assert_eq!(&out[..4], [0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([out[4], out[5]]), 4);
    }
}
