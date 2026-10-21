//! Resize math and EXIF orientation, shared by every raster path.

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::Path;

use image::imageops::FilterType;
use image::metadata::Orientation;
use image::DynamicImage;
use serde::{Deserialize, Serialize};

use super::ConvertError;

/// Decompression-bomb guards: dimension cap plus a hard allocation cap (4 bytes per pixel).
pub const MAX_DIM: u32 = 16_384;
pub const MAX_ALLOC: u64 = 512 * 1024 * 1024;
const MAX_PIXELS: u64 = MAX_ALLOC / 4;
/// EXIF is not read from containers larger than this (the TIFF reader loads the whole file).
const MAX_EXIF_FILE: u64 = 256 * 1024 * 1024;

/// How the box of `Resize::Exact` is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Whole image inside the box, aspect kept (may upscale).
    Contain,
    /// Box fully covered, aspect kept, overflow cropped from the centre.
    Cover,
    /// Exactly the box, aspect ignored.
    Stretch,
}

/// Output size rule. Filter is Lanczos3. `MaxEdge` never upscales; `Percent` and `Exact` may.
/// Vector sources (SVG, PDF) are rendered at the final size directly instead of resampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resize {
    /// Longest edge at most this many pixels (1..=16384).
    MaxEdge(u32),
    /// Percent of the original size (1..=400).
    Percent(u32),
    /// Box of `w` x `h` pixels (each 1..=16384).
    Exact { w: u32, h: u32, fit: Fit },
}

impl Resize {
    pub fn clamped(self) -> Self {
        match self {
            Resize::MaxEdge(p) => Resize::MaxEdge(p.clamp(1, MAX_DIM)),
            Resize::Percent(p) => Resize::Percent(p.clamp(1, 400)),
            Resize::Exact { w, h, fit } => Resize::Exact {
                w: w.clamp(1, MAX_DIM),
                h: h.clamp(1, MAX_DIM),
                fit,
            },
        }
    }

    /// Output size for a `w` x `h` source (rounded, never 0). Not checked against the caps.
    pub fn target(self, w: u32, h: u32) -> (u32, u32) {
        let (w, h) = (u64::from(w.max(1)), u64::from(h.max(1)));
        let scaled = |num: u64, den: u64| {
            (
                ((w * num + den / 2) / den).max(1),
                ((h * num + den / 2) / den).max(1),
            )
        };
        let (tw, th) = match self.clamped() {
            Resize::MaxEdge(p) => {
                let long = w.max(h);
                if long <= u64::from(p) {
                    (w, h)
                } else {
                    scaled(u64::from(p), long)
                }
            }
            Resize::Percent(p) => scaled(u64::from(p), 100),
            Resize::Exact { w: bw, h: bh, fit } => {
                let (bw, bh) = (u64::from(bw), u64::from(bh));
                match fit {
                    Fit::Stretch | Fit::Cover => (bw, bh),
                    // fit inside the box: compare bw/w with bh/h by cross-multiplying
                    Fit::Contain if bw * h <= bh * w => (bw, ((h * bw + w / 2) / w).clamp(1, bh)),
                    Fit::Contain => (((w * bh + h / 2) / h).clamp(1, bw), bh),
                }
            }
        };
        (tw as u32, th as u32)
    }

    /// Scale factor for a vector source of intrinsic size `iw` x `ih` (rendered directly).
    /// `MaxEdge` here means "the longest edge becomes this", upscaling included.
    pub fn vector_scale(self, iw: f32, ih: f32) -> f32 {
        let (iw, ih) = (iw.max(1.0), ih.max(1.0));
        match self.clamped() {
            Resize::MaxEdge(p) => p as f32 / iw.max(ih),
            Resize::Percent(p) => p as f32 / 100.0,
            Resize::Exact { w, h, fit } => {
                let (sx, sy) = (w as f32 / iw, h as f32 / ih);
                match fit {
                    Fit::Contain => sx.min(sy),
                    // Cover and Stretch are finished by `apply`, so render the larger one
                    _ => sx.max(sy),
                }
            }
        }
    }
}

/// Checks a pixel size against the dimension and allocation caps.
pub fn check_size(w: u32, h: u32) -> Result<(), ConvertError> {
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(ConvertError::TooLarge);
    }
    Ok(())
}

/// Resamples to the size `r` asks for.
pub fn apply(img: DynamicImage, r: Resize) -> Result<DynamicImage, ConvertError> {
    let (iw, ih) = (img.width(), img.height());
    let (tw, th) = r.target(iw, ih);
    check_size(tw, th)?;
    if let Resize::Exact {
        w,
        h,
        fit: Fit::Cover,
    } = r.clamped()
    {
        // scale so both edges reach the box, then cut the centre out
        let s = (f64::from(w) / f64::from(iw)).max(f64::from(h) / f64::from(ih));
        let cw = ((f64::from(iw) * s).round() as u32).max(w);
        let ch = ((f64::from(ih) * s).round() as u32).max(h);
        check_size(cw, ch)?;
        let big = resample(&img, cw, ch);
        return Ok(big.crop_imm((cw - w) / 2, (ch - h) / 2, w, h));
    }
    if (tw, th) == (iw, ih) {
        return Ok(img);
    }
    Ok(resample(&img, tw, th))
}

pub fn resample(img: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    img.resize_exact(w, h, FilterType::Lanczos3)
}

/// EXIF orientation (1..=8) of an image file (JPEG, TIFF, PNG, WebP, HEIF containers).
pub fn file_orientation(path: &Path) -> Option<u8> {
    if std::fs::metadata(path).ok()?.len() > MAX_EXIF_FILE {
        return None;
    }
    let mut r = BufReader::new(File::open(path).ok()?);
    orientation_of(exif::Reader::new().read_from_container(&mut r).ok()?)
}

/// Same for a byte prefix of a file (an embedded JPEG).
pub fn bytes_orientation(bytes: &[u8]) -> Option<u8> {
    let mut c = Cursor::new(bytes);
    orientation_of(exif::Reader::new().read_from_container(&mut c).ok()?)
}

fn orientation_of(e: exif::Exif) -> Option<u8> {
    let v = e
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
        .value
        .get_uint(0)?;
    u8::try_from(v).ok().filter(|v| (1..=8).contains(v))
}

/// Bakes the EXIF orientation into the pixels (the output carries no EXIF to say so).
pub fn orient(mut img: DynamicImage, exif_value: Option<u8>) -> DynamicImage {
    if let Some(o) = exif_value.and_then(Orientation::from_exif) {
        img.apply_orientation(o);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn max_edge_never_upscales() {
        assert_eq!(Resize::MaxEdge(100).target(400, 200), (100, 50));
        assert_eq!(Resize::MaxEdge(100).target(200, 400), (50, 100));
        assert_eq!(Resize::MaxEdge(100).target(80, 60), (80, 60));
        assert_eq!(Resize::MaxEdge(100).target(100, 1), (100, 1));
        assert_eq!(Resize::MaxEdge(10).target(1000, 3), (10, 1)); // never 0
        assert_eq!(Resize::MaxEdge(0).target(5, 5), (1, 1)); // clamped to 1
    }

    #[test]
    fn percent_rounds_and_clamps() {
        assert_eq!(Resize::Percent(50).target(101, 51), (51, 26));
        assert_eq!(Resize::Percent(400).target(10, 10), (40, 40));
        assert_eq!(Resize::Percent(9999).target(10, 10), (40, 40));
        assert_eq!(Resize::Percent(0).target(500, 500), (5, 5));
        assert_eq!(Resize::Percent(1).target(10, 10), (1, 1));
    }

    #[test]
    fn exact_fits() {
        let e = |fit| Resize::Exact {
            w: 100,
            h: 100,
            fit,
        };
        assert_eq!(e(Fit::Contain).target(400, 200), (100, 50));
        assert_eq!(e(Fit::Contain).target(200, 400), (50, 100));
        assert_eq!(e(Fit::Contain).target(10, 10), (100, 100)); // upscales
        assert_eq!(e(Fit::Cover).target(400, 200), (100, 100));
        assert_eq!(e(Fit::Stretch).target(3, 7), (100, 100));
    }

    #[test]
    fn vector_scales() {
        assert_eq!(Resize::MaxEdge(200).vector_scale(100.0, 50.0), 2.0);
        assert_eq!(Resize::Percent(150).vector_scale(10.0, 10.0), 1.5);
        let c = Resize::Exact {
            w: 100,
            h: 100,
            fit: Fit::Contain,
        };
        assert_eq!(c.vector_scale(200.0, 100.0), 0.5);
    }

    #[test]
    fn caps_are_enforced() {
        assert!(check_size(16_384, 8_192).is_ok());
        assert!(check_size(16_384, 16_384).is_err()); // 1 GiB of pixels
        assert!(check_size(16_385, 1).is_err());
        assert!(check_size(0, 5).is_err());
        let img = DynamicImage::ImageRgb8(RgbImage::new(10_000, 10_000));
        assert!(matches!(
            apply(img, Resize::Percent(400)),
            Err(ConvertError::TooLarge)
        ));
    }

    #[test]
    fn apply_produces_target_sizes() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(40, 20, |x, _| Rgb([x as u8, 0, 0])));
        let ex = |w, h, fit| Resize::Exact { w, h, fit };
        for (r, want) in [
            (Resize::MaxEdge(20), (20, 10)),
            (Resize::Percent(200), (80, 40)),
            (ex(30, 30, Fit::Cover), (30, 30)),
            (ex(30, 30, Fit::Contain), (30, 15)),
            (ex(7, 9, Fit::Stretch), (7, 9)),
        ] {
            let out = apply(img.clone(), r).unwrap();
            assert_eq!((out.width(), out.height()), want, "{r:?}");
        }
    }

    #[test]
    fn orientation_moves_pixels() {
        // 2x1: left red, right blue. Orientation 6 = rotate 90 clockwise to display.
        let mut src = RgbImage::new(2, 1);
        src.put_pixel(0, 0, Rgb([255, 0, 0]));
        src.put_pixel(1, 0, Rgb([0, 0, 255]));
        let img = DynamicImage::ImageRgb8(src);
        let r = orient(img.clone(), Some(6)).to_rgb8();
        assert_eq!((r.width(), r.height()), (1, 2));
        assert_eq!(r.get_pixel(0, 0), &Rgb([255, 0, 0])); // red ends up on top
        let m = orient(img.clone(), Some(2)).to_rgb8(); // mirrored
        assert_eq!(m.get_pixel(0, 0), &Rgb([0, 0, 255]));
        assert_eq!(orient(img, Some(1)).width(), 2);
        assert_eq!(orient(DynamicImage::new_rgb8(3, 1), Some(9)).width(), 3); // junk ignored
    }

    #[test]
    fn serde_shape() {
        let r: Resize = serde_json::from_str(r#"{"exact":{"w":10,"h":20,"fit":"cover"}}"#).unwrap();
        assert_eq!(
            r,
            Resize::Exact {
                w: 10,
                h: 20,
                fit: Fit::Cover
            }
        );
        assert_eq!(
            serde_json::to_string(&Resize::MaxEdge(5)).unwrap(),
            r#"{"max_edge":5}"#
        );
    }
}
