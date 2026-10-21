//! PDF both ways. Writing: one image per page, page size = image size in points (JPEG
//! photos go in as DCT data, everything else as Flate with an alpha soft mask). Reading:
//! one page rendered by hayro (pure Rust). PDFs are untrusted: the pixel size is capped
//! before rendering and any panic inside the renderer becomes `Corrupt`.

use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf as PdfDoc};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use image::{DynamicImage, RgbaImage};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};

use super::transform::{self, Resize};
use super::ConvertError;

const MAX_FILE: u64 = 256 * 1024 * 1024;
/// Rendering density for a PDF page at 100 percent (points are 1/72 inch).
const DPI: f32 = 150.0;
/// Acrobat's page size limit in points; larger images are scaled down on the page only.
const MAX_PAGE_PT: f32 = 14_400.0;

/// What goes into the PDF stream.
pub enum Payload<'a> {
    /// A ready JPEG file (already cleaned of metadata), 1 or 3 components.
    Jpeg {
        data: &'a [u8],
        w: u32,
        h: u32,
        gray: bool,
    },
    /// Decoded pixels, stored losslessly.
    Pixels(&'a DynamicImage),
}

pub fn write(payload: &Payload, w: &mut impl Write) -> Result<(), ConvertError> {
    write_pages(std::slice::from_ref(payload), w)
}

/// One page per payload, in order.
pub fn write_pages(pages: &[Payload], w: &mut impl Write) -> Result<(), ConvertError> {
    let (catalog, tree) = (Ref::new(1), Ref::new(2));
    let page_ref = |i: usize| Ref::new(3 + 4 * i as i32);
    let mut pdf = Pdf::new();
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree)
        .kids((0..pages.len()).map(page_ref))
        .count(pages.len() as i32);
    for (i, payload) in pages.iter().enumerate() {
        let n = 4 * i as i32;
        let ids = (
            page_ref(i),
            tree,
            Ref::new(4 + n),
            Ref::new(5 + n),
            Ref::new(6 + n),
        );
        write_page(&mut pdf, payload, ids);
    }
    w.write_all(&pdf.finish())?;
    Ok(())
}

fn write_page(pdf: &mut Pdf, payload: &Payload, ids: (Ref, Ref, Ref, Ref, Ref)) {
    let (page, tree, content_id, image_id, mask_id) = ids;
    let (pw, ph) = match payload {
        Payload::Jpeg { w, h, .. } => (*w, *h),
        Payload::Pixels(i) => (i.width(), i.height()),
    };
    let k = (MAX_PAGE_PT / pw.max(ph) as f32).min(1.0);
    let (pt_w, pt_h) = (pw as f32 * k, ph as f32 * k);

    let mut p = pdf.page(page);
    p.media_box(Rect::new(0.0, 0.0, pt_w, pt_h))
        .parent(tree)
        .contents(content_id);
    p.resources().x_objects().pair(Name(b"Im0"), image_id);
    p.finish();

    let mut c = Content::new();
    c.save_state();
    c.transform([pt_w, 0.0, 0.0, pt_h, 0.0, 0.0]);
    c.x_object(Name(b"Im0"));
    c.restore_state();
    pdf.stream(content_id, &c.finish());

    match payload {
        Payload::Jpeg { data, w, h, gray } => {
            let mut x = pdf.image_xobject(image_id, data);
            x.width(*w as i32).height(*h as i32).bits_per_component(8);
            if *gray {
                x.color_space().device_gray();
            } else {
                x.color_space().device_rgb();
            }
            x.filter(Filter::DctDecode);
        }
        Payload::Pixels(img) => {
            let (color, alpha) = split(img);
            let packed = deflate(&color.1);
            let mut x = pdf.image_xobject(image_id, &packed);
            x.width(pw as i32).height(ph as i32).bits_per_component(8);
            if color.0 == 1 {
                x.color_space().device_gray();
            } else {
                x.color_space().device_rgb();
            }
            if alpha.is_some() {
                x.s_mask(mask_id);
            }
            x.filter(Filter::FlateDecode);
            x.finish();
            if let Some(a) = alpha {
                let packed = deflate(&a);
                let mut m = pdf.image_xobject(mask_id, &packed);
                m.width(pw as i32).height(ph as i32).bits_per_component(8);
                m.color_space().device_gray();
                m.filter(Filter::FlateDecode);
            }
        }
    }
}

fn deflate(raw: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(raw, 6)
}

/// ((components, interleaved color bytes), alpha plane if any)
fn split(img: &DynamicImage) -> ((u8, Vec<u8>), Option<Vec<u8>>) {
    if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        let n = rgba.pixels().len();
        let (mut rgb, mut a) = (Vec::with_capacity(n * 3), Vec::with_capacity(n));
        for p in rgba.pixels() {
            rgb.extend_from_slice(&p.0[..3]);
            a.push(p.0[3]);
        }
        ((3, rgb), Some(a))
    } else if img.color().has_color() {
        ((3, img.to_rgb8().into_raw()), None)
    } else {
        ((1, img.to_luma8().into_raw()), None)
    }
}

/// Drops metadata segments from a JPEG (EXIF/XMP/IPTC/comments/maker data), keeping what
/// decoding needs: JFIF, Adobe color flag and, when `keep_icc`, the ICC profile.
/// None when the stream is not a well-formed JPEG head.
pub fn clean_jpeg(src: &[u8], keep_icc: bool) -> Option<Vec<u8>> {
    if src.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2;
    loop {
        if *src.get(i)? != 0xFF {
            return None;
        }
        let m = *src.get(i + 1)?;
        match m {
            0xFF => i += 1, // fill byte
            0x01 | 0xD0..=0xD7 => {
                out.extend_from_slice(&src[i..i + 2]);
                i += 2;
            }
            0xD9 => return None, // EOI before any scan
            _ => {
                let len = usize::from(u16::from_be_bytes([*src.get(i + 2)?, *src.get(i + 3)?]));
                let end = i
                    .checked_add(2 + len)
                    .filter(|&e| len >= 2 && e <= src.len())?;
                let seg = &src[i..end];
                let keep = match m {
                    0xE0 | 0xEE => true,
                    0xE2 => keep_icc && seg[4..].starts_with(b"ICC_PROFILE"),
                    0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
                    _ => true,
                };
                if keep {
                    out.extend_from_slice(seg);
                }
                i = end;
                if m == 0xDA {
                    // entropy-coded data to the end of the file, verbatim
                    out.extend_from_slice(&src[i..]);
                    return Some(out);
                }
            }
        }
    }
}

fn load(src: &Path) -> Result<PdfDoc, ConvertError> {
    if std::fs::metadata(src)?.len() > MAX_FILE {
        return Err(ConvertError::TooLarge);
    }
    let data = std::fs::read(src)?;
    if !data.windows(5).take(1024).any(|w| w == b"%PDF-") {
        return Err(ConvertError::Corrupt);
    }
    PdfDoc::new(data).map_err(|e| match e {
        LoadPdfError::Decryption(_) => ConvertError::Protected,
        LoadPdfError::Invalid => ConvertError::Corrupt,
    })
}

pub fn page_count(src: &Path) -> Result<usize, ConvertError> {
    guarded(|| Ok(load(src)?.pages().len()))
}

/// Renders page `page` (1-based) on white. Without `resize` the page is drawn at 150 dpi;
/// with it, the size rules apply to that and the page is rendered at the final size.
pub fn render_page(
    src: &Path,
    page: u32,
    resize: Option<Resize>,
) -> Result<DynamicImage, ConvertError> {
    let img = guarded(|| {
        let doc = load(src)?;
        let pages = doc.pages();
        let p = pages
            .get((page.max(1) - 1) as usize)
            .ok_or(ConvertError::NoStream)?;
        let (w_pt, h_pt) = p.render_dimensions();
        if !(w_pt.is_finite() && h_pt.is_finite() && w_pt >= 1.0 && h_pt >= 1.0) {
            return Err(ConvertError::Corrupt);
        }
        let base = DPI / 72.0;
        let k = base * resize.map_or(1.0, |r| r.vector_scale(w_pt * base, h_pt * base));
        let (tw, th) = ((w_pt * k).floor(), (h_pt * k).floor());
        if tw > 1e6 || th > 1e6 {
            return Err(ConvertError::TooLarge);
        }
        transform::check_size((tw as u32).max(1), (th as u32).max(1))?;
        let pm = render(
            p,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: k,
                y_scale: k,
                bg_color: WHITE,
            },
        );
        let (w, h) = (u32::from(pm.width()), u32::from(pm.height()));
        // opaque thanks to the white background, so premultiplied equals straight
        RgbaImage::from_raw(w, h, pm.data_as_u8_slice().to_vec())
            .map(DynamicImage::ImageRgba8)
            .ok_or(ConvertError::Corrupt)
    })?;
    match resize {
        Some(r @ Resize::Exact { .. }) => transform::apply(img, r),
        _ => Ok(img),
    }
}

/// Runs the PDF code so that a panic on hostile input is an error, not a crash.
fn guarded<T>(f: impl FnOnce() -> Result<T, ConvertError>) -> Result<T, ConvertError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(Err(ConvertError::Corrupt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn pdf_of(img: &DynamicImage) -> Vec<u8> {
        let mut v = Vec::new();
        write(&Payload::Pixels(img), &mut v).unwrap();
        v
    }

    fn file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.pdf");
        std::fs::write(&p, bytes).unwrap();
        (d, p)
    }

    #[test]
    fn written_pdf_renders_back() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(60, 30, |x, _| {
            if x < 30 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        }));
        let (_d, p) = file(&pdf_of(&img));
        assert_eq!(page_count(&p).unwrap(), 1);
        let out = render_page(&p, 1, None).unwrap().to_rgba8();
        // 60x30 pt at 150 dpi (the renderer truncates)
        assert!((124..=125).contains(&out.width()) && out.height() == 62);
        let l = out.get_pixel(10, 30).0;
        let r = out.get_pixel(114, 30).0;
        assert!(l[0] > 200 && l[2] < 60, "{l:?}");
        assert!(r[2] > 200 && r[0] < 60, "{r:?}");
    }

    #[test]
    fn alpha_becomes_a_mask_and_renders_white() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(20, 20, Rgba([0, 0, 0, 0])));
        let (_d, p) = file(&pdf_of(&img));
        let out = render_page(&p, 1, None).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(5, 5).0, [255, 255, 255, 255]);
    }

    #[test]
    fn page_size_matches_image_and_resize_applies() {
        let img = DynamicImage::new_rgb8(200, 100);
        let (_d, p) = file(&pdf_of(&img));
        assert_eq!(
            render_page(&p, 1, Some(Resize::MaxEdge(100)))
                .unwrap()
                .width(),
            100
        );
        assert_eq!(
            render_page(&p, 1, Some(Resize::Percent(50)))
                .unwrap()
                .width(),
            208
        );
        assert!(matches!(
            render_page(&p, 2, None),
            Err(ConvertError::NoStream)
        ));
    }

    #[test]
    fn jpeg_passthrough_embeds_the_original_bytes() {
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 80)
            .encode_image(&DynamicImage::new_rgb8(32, 16).to_rgb8())
            .unwrap();
        let mut v = Vec::new();
        write(
            &Payload::Jpeg {
                data: &jpg,
                w: 32,
                h: 16,
                gray: false,
            },
            &mut v,
        )
        .unwrap();
        assert!(v.windows(jpg.len()).any(|w| w == jpg.as_slice()));
        let (_d, p) = file(&v);
        assert!((65..=66).contains(&render_page(&p, 1, None).unwrap().width()));
    }

    #[test]
    fn clean_jpeg_drops_exif_xmp_comments_but_decodes() {
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpg)
            .encode_image(&DynamicImage::new_rgb8(16, 16).to_rgb8())
            .unwrap();
        // splice EXIF (APP1), ICC (APP2) and a comment right after SOI
        let seg = |m: u8, body: &[u8]| {
            let mut s = vec![0xFF, m];
            s.extend_from_slice(&(body.len() as u16 + 2).to_be_bytes());
            s.extend_from_slice(body);
            s
        };
        let mut dirty = vec![0xFF, 0xD8];
        dirty.extend(seg(0xE1, b"Exif\0\0GPSSECRET"));
        dirty.extend(seg(0xE2, b"ICC_PROFILE\0\x01\x01data"));
        dirty.extend(seg(0xFE, b"my comment"));
        dirty.extend_from_slice(&jpg[2..]);
        let has = |b: &[u8], n: &[u8]| b.windows(n.len()).any(|w| w == n);
        let strip = clean_jpeg(&dirty, false).unwrap();
        assert!(
            !has(&strip, b"GPSSECRET") && !has(&strip, b"ICC_PROFILE") && !has(&strip, b"comment")
        );
        let keep = clean_jpeg(&dirty, true).unwrap();
        assert!(!has(&keep, b"GPSSECRET") && has(&keep, b"ICC_PROFILE"));
        for out in [strip, keep] {
            assert_eq!(image::load_from_memory(&out).unwrap().width(), 16);
        }
        assert!(clean_jpeg(b"nope", false).is_none());
        assert!(clean_jpeg(&dirty[..10], false).is_none());
    }

    #[test]
    fn garbage_is_an_error_never_a_panic() {
        let cases: [&[u8]; 6] = [
            b"%PDF-1.4\n garbage",
            b"%PDF-1.7\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\ntrailer<</Root 1 0 R>>",
            b"not a pdf at all",
            &[0u8; 64],
            b"%PDF-1.4\n1 0 obj<</Type/Pages/Kids[1 0 R]/Count 1>>endobj\ntrailer<</Root 1 0 R>>",
            b"%PDF-",
        ];
        for c in cases {
            let (_d, p) = file(c);
            assert!(render_page(&p, 1, None).is_err());
            let _ = page_count(&p);
        }
    }

    #[test]
    fn oversized_page_is_refused_before_rendering() {
        // a page of 30000x30000 pt would be 62500 px a side
        let mut pdf = Pdf::new();
        let (cat, tree, page) = (Ref::new(1), Ref::new(2), Ref::new(3));
        pdf.catalog(cat).pages(tree);
        pdf.pages(tree).kids([page]).count(1);
        pdf.page(page)
            .media_box(Rect::new(0.0, 0.0, 30_000.0, 30_000.0))
            .parent(tree);
        let (_d, p) = file(&pdf.finish());
        assert!(matches!(
            render_page(&p, 1, None),
            Err(ConvertError::TooLarge)
        ));
    }
}
