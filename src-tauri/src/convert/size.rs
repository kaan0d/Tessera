//! Compress-to-target-size search for still images: binary search on quality for lossy
//! encoders, then progressive downscaling from the original (never cumulative blur).

use image::DynamicImage;

use super::{ConvertError, transform};

/// Smallest long edge the downscale loop will go to.
const MIN_EDGE: u32 = 128;
/// Downscale rounds before giving up.
const MAX_ROUNDS: usize = 12;

/// Bisects `1..=q0` for the highest quality whose output fits `max`; None when even
/// quality 1 does not.
fn best_quality<E>(
    q0: u8,
    max: u64,
    enc: &mut impl FnMut(u8) -> Result<Vec<u8>, E>,
) -> Result<Option<Vec<u8>>, E> {
    let fits = |b: &Vec<u8>| b.len() as u64 <= max;
    let top = enc(q0)?;
    if fits(&top) {
        return Ok(Some(top));
    }
    if q0 <= 1 {
        return Ok(None);
    }
    let (mut lo, mut hi) = (1u8, q0 - 1);
    let mut best = None;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let out = enc(mid)?;
        if fits(&out) {
            best = Some(out);
            lo = mid + 1;
        } else if mid == 1 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    Ok(best)
}

/// Encodes `img` so the result is at most `max` bytes.
///
/// `lossy` encoders get their quality searched in `1..=q0` (the best fit wins); a lossless
/// one, or a lossy one at quality 1, shrinks the image instead. `enc(image, quality)` must
/// return the whole encoded file. Fails with `TargetSize` when the long edge would drop
/// below 160 px or after 12 rounds.
pub fn fit(
    img: &DynamicImage,
    max: u64,
    lossy: bool,
    q0: u8,
    mut enc: impl FnMut(&DynamicImage, u8) -> Result<Vec<u8>, ConvertError>,
) -> Result<Vec<u8>, ConvertError> {
    let (w0, h0) = (img.width(), img.height());
    let mut scale = 1.0f64;
    let mut cur;
    let mut view = img;
    for _ in 0..MAX_ROUNDS {
        let (size, found) = if lossy {
            match best_quality(q0.clamp(1, 100), max, &mut |q| enc(view, q))? {
                Some(b) => (0, Some(b)),
                // quality 1 is the floor: measure it for the shrink estimate
                None => (enc(view, 1)?.len() as u64, None),
            }
        } else {
            let b = enc(view, 100)?;
            if b.len() as u64 <= max {
                if view.width() != w0 {
                    let wh = u64::from(view.width()) << 32 | u64::from(view.height());
                    super::SHRUNK_TO.store(wh, std::sync::atomic::Ordering::Relaxed);
                }
                return Ok(b);
            }
            (b.len() as u64, None)
        };
        if let Some(b) = found {
            if view.width() != w0 {
                let wh = u64::from(view.width()) << 32 | u64::from(view.height());
                super::SHRUNK_TO.store(wh, std::sync::atomic::Ordering::Relaxed);
            }
            return Ok(b);
        }
        // bytes scale roughly with area; aim a bit under the limit so the next round fits
        let ratio = ((max as f64 / size.max(1) as f64).sqrt() * 0.92).clamp(0.2, 0.93);
        scale *= ratio;
        let (nw, nh) = (
            ((f64::from(w0) * scale).round() as u32).max(1),
            ((f64::from(h0) * scale).round() as u32).max(1),
        );
        if nw.max(nh) < MIN_EDGE.min(w0.max(h0)) {
            break;
        }
        cur = transform::resample(img, nw, nh);
        view = &cur;
    }
    Err(ConvertError::TargetSize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, codecs::jpeg::JpegEncoder};

    fn noisy(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            let n = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503);
            Rgb([(n >> 3) as u8, (n >> 11) as u8, ((x + y) * 3) as u8])
        }))
    }

    fn jpeg(img: &DynamicImage, q: u8) -> Result<Vec<u8>, ConvertError> {
        let mut v = Vec::new();
        JpegEncoder::new_with_quality(&mut v, q).encode_image(&img.to_rgb8())?;
        Ok(v)
    }

    #[test]
    fn already_small_keeps_top_quality() {
        let img = noisy(64, 64);
        let big = jpeg(&img, 90).unwrap();
        let out = fit(&img, big.len() as u64 + 10, true, 90, jpeg).unwrap();
        assert_eq!(out, big);
    }

    #[test]
    fn quality_search_finds_the_best_fit() {
        let img = noisy(200, 200);
        let (hi, lo) = (jpeg(&img, 90).unwrap().len(), jpeg(&img, 1).unwrap().len());
        let max = ((hi + lo) / 2) as u64;
        let mut calls = 0;
        let out = fit(&img, max, true, 90, |i, q| {
            calls += 1;
            jpeg(i, q)
        })
        .unwrap();
        assert!(out.len() as u64 <= max);
        assert!(calls <= 9, "bisection, not a scan: {calls}");
        // still full size: only the quality moved
        assert_eq!(image::load_from_memory(&out).unwrap().width(), 200);
        // one quality step higher would not fit
        let q_used = (1..=90u8)
            .rev()
            .find(|&q| jpeg(&img, q).unwrap().len() as u64 <= max)
            .unwrap();
        assert_eq!(out, jpeg(&img, q_used).unwrap());
    }

    #[test]
    fn shrinks_when_quality_is_not_enough() {
        let img = noisy(400, 400);
        let floor = jpeg(&img, 1).unwrap().len() as u64;
        let max = floor / 4;
        let out = fit(&img, max, true, 90, jpeg).unwrap();
        assert!(out.len() as u64 <= max);
        assert!(image::load_from_memory(&out).unwrap().width() < 400);
    }

    #[test]
    fn lossless_shrinks_by_size() {
        let img = noisy(600, 600);
        let png = |i: &DynamicImage, _q| {
            let mut c = std::io::Cursor::new(Vec::new());
            i.write_to(&mut c, image::ImageFormat::Png)?;
            Ok(c.into_inner())
        };
        let full = png(&img, 100).unwrap().len() as u64;
        let out = fit(&img, full / 3, false, 90, png).unwrap();
        assert!(out.len() as u64 <= full / 3);
        assert!(image::load_from_memory(&out).unwrap().width() < 600);
    }

    #[test]
    fn impossible_target_is_an_error() {
        let img = noisy(300, 300);
        assert!(matches!(
            fit(&img, 10, true, 90, jpeg),
            Err(ConvertError::TargetSize)
        ));
        assert!(matches!(
            fit(&img, 10, false, 90, jpeg),
            Err(ConvertError::TargetSize)
        ));
    }
}
