//! SVG source: rasterized with resvg. The SVG is untrusted input, so external files and
//! URLs are never loaded (only `data:` images inline), the size is capped before any
//! pixel buffer exists, and the file itself is size-limited.

use std::path::Path;
use std::sync::Arc;

use image::{DynamicImage, RgbaImage};
use resvg::usvg::{self, ImageHrefResolver};

use super::transform::{self, Resize};
use super::ConvertError;

const MAX_FILE: u64 = 64 * 1024 * 1024;

pub fn render(src: &Path, resize: Option<Resize>) -> Result<DynamicImage, ConvertError> {
    if std::fs::metadata(src)?.len() > MAX_FILE {
        return Err(ConvertError::TooLarge);
    }
    render_bytes(&std::fs::read(src)?, resize)
}

pub fn render_bytes(data: &[u8], resize: Option<Resize>) -> Result<DynamicImage, ConvertError> {
    let mut opt = usvg::Options {
        image_href_resolver: ImageHrefResolver {
            resolve_string: Box::new(|_, _| None), // no local files, no network
            ..ImageHrefResolver::default()
        },
        ..usvg::Options::default()
    };
    // scanning system fonts costs time; only SVGs with text need them
    if data.windows(5).any(|w| w == b"<text") {
        Arc::make_mut(&mut opt.fontdb).load_system_fonts();
    }
    let tree = usvg::Tree::from_data(data, &opt).map_err(|_| ConvertError::Corrupt)?;
    let (iw, ih) = (tree.size().width(), tree.size().height());
    if !(iw.is_finite() && ih.is_finite() && iw >= 1.0 && ih >= 1.0) {
        return Err(ConvertError::Corrupt);
    }
    let s = resize.map_or(1.0, |r| r.vector_scale(iw, ih));
    let (tw, th) = ((iw * s).round(), (ih * s).round());
    if !(tw.is_finite() && th.is_finite()) || tw > 1e6 || th > 1e6 {
        return Err(ConvertError::TooLarge);
    }
    let (tw, th) = ((tw as u32).max(1), (th as u32).max(1));
    transform::check_size(tw, th)?;

    let mut pm = resvg::tiny_skia::Pixmap::new(tw, th).ok_or(ConvertError::TooLarge)?;
    let t = resvg::tiny_skia::Transform::from_scale(tw as f32 / iw, th as f32 / ih);
    resvg::render(&tree, t, &mut pm.as_mut());

    let mut raw = Vec::with_capacity(pm.pixels().len() * 4);
    for p in pm.pixels() {
        let c = p.demultiply();
        raw.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let img =
        DynamicImage::ImageRgba8(RgbaImage::from_raw(tw, th, raw).ok_or(ConvertError::Corrupt)?);
    // Exact boxes with Cover/Stretch (and rounding) are finished on pixels
    match resize {
        Some(r @ Resize::Exact { .. }) => transform::apply(img, r),
        _ => Ok(img),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::Fit;

    const RED_BLUE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
        <rect width="20" height="20" fill="#ff0000"/><rect x="20" width="20" height="20" fill="#0000ff"/></svg>"##;

    #[test]
    fn renders_at_intrinsic_size_and_colors() {
        let img = render_bytes(RED_BLUE, None).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (40, 20));
        assert_eq!(img.get_pixel(5, 5).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(30, 5).0, [0, 0, 255, 255]);
    }

    #[test]
    fn scale_options_render_natively() {
        let w = |r| render_bytes(RED_BLUE, Some(r)).unwrap().width();
        assert_eq!(w(Resize::Percent(300)), 120);
        assert_eq!(w(Resize::MaxEdge(200)), 200); // vector: upscales
        let img = render_bytes(
            RED_BLUE,
            Some(Resize::Exact {
                w: 50,
                h: 50,
                fit: Fit::Contain,
            }),
        )
        .unwrap();
        assert_eq!((img.width(), img.height()), (50, 25));
        let img = render_bytes(
            RED_BLUE,
            Some(Resize::Exact {
                w: 30,
                h: 30,
                fit: Fit::Cover,
            }),
        )
        .unwrap();
        assert_eq!((img.width(), img.height()), (30, 30));
    }

    #[test]
    fn transparency_is_kept() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;
        assert_eq!(
            render_bytes(svg, None)
                .unwrap()
                .to_rgba8()
                .get_pixel(1, 1)
                .0[3],
            0
        );
    }

    #[test]
    fn huge_and_degenerate_sizes_are_refused() {
        let huge = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000"/>"#;
        assert!(matches!(
            render_bytes(huge, None),
            Err(ConvertError::TooLarge)
        ));
        let big_scale = render_bytes(RED_BLUE, Some(Resize::Percent(400)));
        assert!(big_scale.is_ok());
        let zero = br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#;
        assert!(render_bytes(zero, None).is_err());
    }

    #[test]
    fn external_images_are_not_loaded() {
        let d = tempfile::tempdir().unwrap();
        let secret = d.path().join("secret.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 255, 0, 255]))
            .save(&secret)
            .unwrap();
        let href = secret.to_string_lossy().replace('\\', "/");
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><image width="8" height="8" xlink:href="{href}"/></svg>"#
        );
        let img = render_bytes(svg.as_bytes(), None).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(4, 4).0[3], 0, "local file must not be drawn");
    }

    #[test]
    fn garbage_is_corrupt_not_a_panic() {
        for g in [
            &b"not xml"[..],
            b"<svg",
            b"\xff\xfe\x00",
            b"<svg xmlns='http://www.w3.org/2000/svg'><",
        ] {
            assert!(render_bytes(g, None).is_err());
        }
        // entity bomb
        let bomb = br#"<?xml version="1.0"?><!DOCTYPE s [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;"><!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;"><!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;">]><svg xmlns="http://www.w3.org/2000/svg" width="5" height="5"><text>&d;&d;&d;&d;</text></svg>"#;
        let _ = render_bytes(bomb, None);
    }
}
