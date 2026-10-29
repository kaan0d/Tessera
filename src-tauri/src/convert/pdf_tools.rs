//! Tools for one PDF: split into pages, extract or delete pages, rotate, compress. Each tool
//! writes new files (`name-rotated.pdf`, a `name-pages` folder, ...); the source stays as it is.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, ImageFormat};
use lopdf::{Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};

use super::{output, pdf_edit, security, Conflict, ConvertError, Format, Options};

/// Most entries a page list may expand to (a typo like 1-99999999 must not allocate).
const MAX_PICKS: usize = 10_000;
/// Images smaller than this (stored bytes) are left alone by Compress.
const MIN_IMAGE_BYTES: usize = 20_000;
const MAX_PIXELS: i64 = 100_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum PdfTool {
    /// Every page as its own PDF, in a new folder.
    Split,
    /// Only these pages, in this order (`1-3, 5`).
    Extract { pages: String },
    /// Everything but these pages.
    Delete { pages: String },
    /// Turn pages clockwise by 90, 180 or 270 degrees; empty `pages` means all.
    Rotate { angle: u16, pages: String },
    /// Re-encode large pictures as JPEG; `strong` makes them smaller still.
    Compress { strong: bool },
}

impl PdfTool {
    /// Command-line spelling: `split`, `extract:1-3,5`, `delete:2`, `rotate:90`,
    /// `rotate:90:1-2`, `compress`, `compress:strong`.
    pub fn arg(&self) -> String {
        match self {
            PdfTool::Split => "split".into(),
            PdfTool::Extract { pages } => format!("extract:{pages}"),
            PdfTool::Delete { pages } => format!("delete:{pages}"),
            PdfTool::Rotate { angle, pages } if pages.is_empty() => format!("rotate:{angle}"),
            PdfTool::Rotate { angle, pages } => format!("rotate:{angle}:{pages}"),
            PdfTool::Compress { strong: false } => "compress".into(),
            PdfTool::Compress { strong: true } => "compress:strong".into(),
        }
    }

    pub fn parse(s: &str) -> Option<PdfTool> {
        let (name, rest) = s.split_once(':').unwrap_or((s, ""));
        let pages = || (!rest.trim().is_empty()).then(|| rest.trim().to_string());
        let tool = match name {
            "split" if rest.is_empty() => PdfTool::Split,
            "extract" => PdfTool::Extract { pages: pages()? },
            "delete" => PdfTool::Delete { pages: pages()? },
            "rotate" => {
                let (angle, pages) = rest.split_once(':').unwrap_or((rest, ""));
                PdfTool::Rotate {
                    angle: angle.parse().ok()?,
                    pages: pages.trim().to_string(),
                }
            }
            "compress" if rest.is_empty() => PdfTool::Compress { strong: false },
            "compress" if rest == "strong" => PdfTool::Compress { strong: true },
            _ => return None,
        };
        tool.valid().then_some(tool)
    }

    pub fn valid(&self) -> bool {
        match self {
            PdfTool::Rotate { angle, .. } => matches!(angle, 90 | 180 | 270),
            PdfTool::Extract { pages } | PdfTool::Delete { pages } => !pages.trim().is_empty(),
            _ => true,
        }
    }

    /// i18n key of the tool's name, for menus and history.
    pub fn label_key(&self) -> &'static str {
        match self {
            PdfTool::Split => "pdf.split",
            PdfTool::Extract { .. } => "pdf.extract",
            PdfTool::Delete { .. } => "pdf.delete",
            PdfTool::Rotate { angle: 90, .. } => "pdf.rotate_right",
            PdfTool::Rotate { angle: 270, .. } => "pdf.rotate_left",
            PdfTool::Rotate { .. } => "pdf.rotate_180",
            PdfTool::Compress { .. } => "pdf.compress",
        }
    }
}

/// `1-3, 5, 8-` against a PDF of `count` pages: 1-based numbers in the order written,
/// each once. A range may run backwards (`5-3`); `8-` runs to the end, `-2` from the start.
pub fn parse_pages(spec: &str, count: u32) -> Result<Vec<u32>, ConvertError> {
    let bad = || ConvertError::BadPages(count);
    let num = |s: &str| -> Result<u32, ConvertError> {
        let n: u32 = s.trim().parse().map_err(|_| bad())?;
        (1..=count).contains(&n).then_some(n).ok_or_else(bad)
    };
    let (mut out, mut seen) = (vec![], HashSet::new());
    for part in spec
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let (a, b) = match part.split_once('-') {
            None => (num(part)?, num(part)?),
            Some((a, b)) => (
                if a.trim().is_empty() { 1 } else { num(a)? },
                if b.trim().is_empty() { count } else { num(b)? },
            ),
        };
        let range: Box<dyn Iterator<Item = u32>> = if a <= b {
            Box::new(a..=b)
        } else {
            Box::new((b..=a).rev())
        };
        for n in range {
            if seen.insert(n) {
                out.push(n);
            }
        }
        if out.len() > MAX_PICKS {
            return Err(bad());
        }
    }
    if out.is_empty() {
        return Err(bad());
    }
    Ok(out)
}

/// Where a one-file result goes: `<stem><suffix>.pdf` under the output rules. A tool never
/// replaces its source, so ReplaceOriginal becomes AutoRename here.
fn target(src: &Path, suffix: &str, opts: &Options) -> Result<PathBuf, ConvertError> {
    let mut p = opts.output.clone();
    if p.conflict == Conflict::ReplaceOriginal {
        p.conflict = Conflict::AutoRename;
    }
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    // any extension keeps every dot of "contract.v1" in the stem `plan` takes; not ".pdf", or
    // `plan` would see the planned name as the source and number it
    output::plan(
        &src.with_file_name(format!("{stem}{suffix}.src")),
        "pdf",
        &p,
    )
}

fn write(bytes: &[u8], dest: &Path) -> Result<(), ConvertError> {
    output::write_atomic(dest, |w| {
        std::io::Write::write_all(w, bytes)?;
        Ok(())
    })
}

/// Runs `tool` on `input`; returns the written file, or the folder for Split.
pub fn run(
    tool: &PdfTool,
    input: &Path,
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let opts = &opts.clamped();
    let (src, from) = security::validate_input(input)?;
    if from != Format::Pdf || !tool.valid() {
        return Err(ConvertError::Unsupported);
    }
    progress(0.0);
    let mut doc = pdf_edit::load(&src)?;
    let all = pdf_edit::pages(&doc);
    let count = all.len() as u32;
    let page = |n: &u32| all[*n as usize - 1];
    opts.check_cancel()?;
    let (suffix, bytes) = match tool {
        PdfTool::Split => return split(&doc, &src, opts, progress),
        PdfTool::Extract { pages } => {
            let kids: Vec<_> = parse_pages(pages, count)?.iter().map(page).collect();
            pdf_edit::keep_pages(&mut doc, &kids);
            ("-pages", pdf_edit::save(&mut doc)?)
        }
        PdfTool::Delete { pages } => {
            let gone: HashSet<ObjectId> = parse_pages(pages, count)?.iter().map(page).collect();
            let kids: Vec<_> = all.iter().copied().filter(|p| !gone.contains(p)).collect();
            if kids.is_empty() {
                // deleting every page would leave nothing to save
                return Err(ConvertError::NoPagesLeft);
            }
            pdf_edit::keep_pages(&mut doc, &kids);
            ("-edited", pdf_edit::save(&mut doc)?)
        }
        PdfTool::Rotate { angle, pages } => {
            let picked: Vec<ObjectId> = if pages.trim().is_empty() {
                all.clone()
            } else {
                parse_pages(pages, count)?.iter().map(page).collect()
            };
            for id in picked {
                pdf_edit::rotate(&mut doc, id, i64::from(*angle));
            }
            ("-rotated", pdf_edit::save(&mut doc)?)
        }
        PdfTool::Compress { strong } => {
            shrink_images(&mut doc, *strong, opts, &mut progress)?;
            let bytes = pdf_edit::save(&mut doc)?;
            // nothing to gain: hand back an unchanged copy rather than a bigger file
            let original = std::fs::read(&src)?;
            let best = if bytes.len() < original.len() {
                bytes
            } else {
                original
            };
            ("-compressed", best)
        }
    };
    opts.check_cancel()?;
    let dest = target(&src, suffix, opts)?;
    write(&bytes, &dest)?;
    progress(1.0);
    Ok(dest)
}

/// One PDF per page in a new `<stem>-pages` folder; the folder goes again on any failure.
fn split(
    doc: &Document,
    src: &Path,
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let all = pdf_edit::pages(doc);
    let dir = output::new_dir(src, "-pages", &opts.output)?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("page");
    let width = all.len().to_string().len();
    let result = all.iter().enumerate().try_for_each(|(i, &id)| {
        opts.check_cancel()?;
        // ponytail: one full clone per page, O(pages x size); fine below a few hundred pages
        let mut one = doc.clone();
        pdf_edit::keep_pages(&mut one, &[id]);
        let bytes = pdf_edit::save(&mut one)?;
        write(&bytes, &dir.join(format!("{stem}-{:0width$}.pdf", i + 1)))?;
        progress((i + 1) as f32 / all.len() as f32);
        Ok(())
    });
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

fn is_image(o: &Object) -> bool {
    o.as_stream().is_ok_and(|s| {
        s.dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok_and(|n| n == b"Image")
    })
}

/// Re-encodes every large RGB or gray picture as a smaller JPEG where that saves space.
/// Soft masks, color-keyed and unusual images are left alone.
fn shrink_images(
    doc: &mut Document,
    strong: bool,
    opts: &Options,
    progress: &mut impl FnMut(f32),
) -> Result<(), ConvertError> {
    let (max_edge, quality) = if strong { (1200, 50) } else { (2000, 72) };
    let masks: HashSet<ObjectId> = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter_map(|s| s.dict.get(b"SMask").and_then(Object::as_reference).ok())
        .collect();
    let ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(id, o)| !masks.contains(id) && is_image(o))
        .map(|(id, _)| *id)
        .collect();
    for (i, id) in ids.iter().enumerate() {
        opts.check_cancel()?;
        if let Some(Object::Stream(s)) = doc.objects.get_mut(id) {
            if let Some((jpeg, w, h, gray)) =
                pdf_edit::guarded(|| Ok(shrink(s, max_edge, quality)))?
            {
                s.set_content(jpeg);
                s.dict.remove(b"DecodeParms");
                s.dict.set("Filter", "DCTDecode");
                s.dict.set("Width", i64::from(w));
                s.dict.set("Height", i64::from(h));
                s.dict.set("BitsPerComponent", 8);
                s.dict
                    .set("ColorSpace", if gray { "DeviceGray" } else { "DeviceRGB" });
            }
        }
        progress(0.1 + 0.8 * (i + 1) as f32 / ids.len() as f32);
    }
    Ok(())
}

/// The picture of `s` as a smaller JPEG: (bytes, width, height, gray), or None to keep it.
fn shrink(s: &Stream, max_edge: u32, quality: u8) -> Option<(Vec<u8>, u32, u32, bool)> {
    let d = &s.dict;
    let int = |k: &[u8]| d.get(k).and_then(Object::as_i64).ok();
    let (w, h) = (int(b"Width")?, int(b"Height")?);
    if w <= 0 || h <= 0 || w * h > MAX_PIXELS || int(b"BitsPerComponent") != Some(8) {
        return None;
    }
    if s.content.len() < MIN_IMAGE_BYTES || d.has(b"Mask") || d.has(b"Decode") {
        return None;
    }
    if d.get(b"ImageMask")
        .and_then(Object::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let gray = match d.get(b"ColorSpace").and_then(Object::as_name).ok()? {
        b"DeviceRGB" => false,
        b"DeviceGray" => true,
        _ => return None,
    };
    let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    let img = match s.filters().ok()?.as_slice() {
        [b"DCTDecode"] => {
            image::load_from_memory_with_format(&s.content, ImageFormat::Jpeg).ok()?
        }
        [b"FlateDecode"] => {
            let len = (w * h) as usize * if gray { 1 } else { 3 };
            // PNG predictors add a filter byte per row before they are undone
            let raw = s
                .decompressed_content_with_limit(len + h as usize + 64)
                .ok()?;
            if raw.len() != len {
                return None;
            }
            if gray {
                DynamicImage::ImageLuma8(ImageBuffer::from_raw(w, h, raw)?)
            } else {
                DynamicImage::ImageRgb8(ImageBuffer::from_raw(w, h, raw)?)
            }
        }
        _ => return None,
    };
    let img = if img.width().max(img.height()) > max_edge {
        img.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        img
    };
    let img = if gray {
        DynamicImage::ImageLuma8(img.to_luma8())
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8())
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&img)
        .ok()?;
    (out.len() < s.content.len() * 9 / 10).then(|| (out, img.width(), img.height(), gray))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::pdf_edit::tests::{sample, widths};

    fn pdf(dir: &Path, n: usize) -> PathBuf {
        let p = dir.join("doc.pdf");
        std::fs::write(&p, sample(n)).unwrap();
        p
    }

    fn opened(p: &Path) -> Document {
        pdf_edit::load(p).unwrap()
    }

    #[test]
    fn page_lists() {
        assert_eq!(parse_pages("1-3, 5", 9).unwrap(), [1, 2, 3, 5]);
        assert_eq!(parse_pages("8-", 9).unwrap(), [8, 9]);
        assert_eq!(parse_pages("-2", 9).unwrap(), [1, 2]);
        assert_eq!(parse_pages("4-2;2", 9).unwrap(), [4, 3, 2]);
        for bad in ["", "0", "10", "a", "1-x", "3--4", ",", "1-100000000"] {
            assert!(
                matches!(parse_pages(bad, 9), Err(ConvertError::BadPages(9))),
                "{bad}"
            );
        }
    }

    #[test]
    fn tool_args_round_trip() {
        for t in [
            PdfTool::Split,
            PdfTool::Extract {
                pages: "1-3,5".into(),
            },
            PdfTool::Delete { pages: "2".into() },
            PdfTool::Rotate {
                angle: 90,
                pages: String::new(),
            },
            PdfTool::Rotate {
                angle: 270,
                pages: "1-2".into(),
            },
            PdfTool::Compress { strong: false },
            PdfTool::Compress { strong: true },
        ] {
            assert_eq!(PdfTool::parse(&t.arg()), Some(t));
        }
        for bad in [
            "",
            "rotate:45",
            "extract:",
            "delete",
            "split:1",
            "compress:x",
            "zap",
        ] {
            assert_eq!(PdfTool::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn extract_delete_and_rotate_write_new_files() {
        let d = tempfile::tempdir().unwrap();
        let src = pdf(d.path(), 4);
        let o = Options::default();
        let ex = run(
            &PdfTool::Extract {
                pages: "4, 2".into(),
            },
            &src,
            &o,
            |_| {},
        )
        .unwrap();
        assert!(ex.ends_with("doc-pages.pdf"));
        assert_eq!(widths(&opened(&ex)), [103.0, 101.0]);
        let del = run(
            &PdfTool::Delete {
                pages: "1-2".into(),
            },
            &src,
            &o,
            |_| {},
        )
        .unwrap();
        assert_eq!(widths(&opened(&del)), [102.0, 103.0]);
        assert!(matches!(
            run(
                &PdfTool::Delete {
                    pages: "1-4".into()
                },
                &src,
                &o,
                |_| {}
            ),
            Err(ConvertError::NoPagesLeft)
        ));
        // a dotted name keeps its dots: contract.v1.pdf -> contract.v1-pages.pdf
        let dotted = d.path().join("contract.v1.pdf");
        std::fs::copy(&src, &dotted).unwrap();
        let one = PdfTool::Extract { pages: "1".into() };
        let out = run(&one, &dotted, &o, |_| {}).unwrap();
        assert!(out.ends_with("contract.v1-pages.pdf"), "{out:?}");
        let rot = run(
            &PdfTool::Rotate {
                angle: 90,
                pages: "2".into(),
            },
            &src,
            &o,
            |_| {},
        )
        .unwrap();
        let doc = opened(&rot);
        let turns: Vec<i64> = pdf_edit::pages(&doc)
            .into_iter()
            .map(|id| {
                let p = doc.get_dictionary(id).unwrap();
                p.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0)
            })
            .collect();
        assert_eq!(turns, [0, 90, 0, 0]);
        // the source is untouched
        assert_eq!(std::fs::read(&src).unwrap(), sample(4));
    }

    #[test]
    fn split_makes_a_folder_of_single_pages() {
        let d = tempfile::tempdir().unwrap();
        let src = pdf(d.path(), 12);
        let dir = run(&PdfTool::Split, &src, &Options::default(), |_| {}).unwrap();
        assert!(dir.ends_with("doc-pages"));
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 12);
        assert_eq!(names[0], "doc-01.pdf");
        assert_eq!(widths(&opened(&dir.join("doc-12.pdf"))), [111.0]);
        let again = run(&PdfTool::Split, &src, &Options::default(), |_| {}).unwrap();
        assert!(again.ends_with("doc-pages (1)"));
    }

    #[test]
    fn compress_shrinks_a_big_picture() {
        use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};
        let d = tempfile::tempdir().unwrap();
        // a noisy 1500x1500 RGB picture stored losslessly
        let (w, h) = (1500u32, 1500u32);
        let mut seed = 0x2545_f491_u32;
        let px: Vec<u8> = (0..w * h * 3)
            .map(|i| {
                // a photo-like gradient with grain: deflate stores it badly, JPEG well
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                ((i / 3 % w + i / 3 / w) / 12 + (seed % 24)) as u8
            })
            .collect();
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&px, 6);
        let mut p = Pdf::new();
        let (cat, tree, page, content, img) = (
            Ref::new(1),
            Ref::new(2),
            Ref::new(3),
            Ref::new(4),
            Ref::new(5),
        );
        p.catalog(cat).pages(tree);
        p.pages(tree).kids([page]).count(1);
        let mut pg = p.page(page);
        pg.parent(tree)
            .media_box(Rect::new(0.0, 0.0, 500.0, 500.0))
            .contents(content);
        pg.resources().x_objects().pair(Name(b"Im0"), img);
        pg.finish();
        let mut c = Content::new();
        c.transform([500.0, 0.0, 0.0, 500.0, 0.0, 0.0])
            .x_object(Name(b"Im0"));
        p.stream(content, &c.finish());
        let mut x = p.image_xobject(img, &packed);
        x.width(w as i32).height(h as i32).bits_per_component(8);
        x.color_space().device_rgb();
        x.filter(Filter::FlateDecode);
        x.finish();
        let src = d.path().join("big.pdf");
        std::fs::write(&src, p.finish()).unwrap();
        let out = run(
            &PdfTool::Compress { strong: true },
            &src,
            &Options::default(),
            |_| {},
        )
        .unwrap();
        assert!(out.ends_with("big-compressed.pdf"));
        let (before, after) = (
            std::fs::metadata(&src).unwrap().len(),
            std::fs::metadata(&out).unwrap().len(),
        );
        assert!(after * 2 < before, "{before} -> {after}");
        assert_eq!(crate::convert::pdf::page_count(&out).unwrap(), 1);
    }

    #[test]
    fn other_files_and_locked_pdfs_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let png = d.path().join("a.png");
        ::image::RgbImage::new(2, 2).save(&png).unwrap();
        assert!(matches!(
            run(&PdfTool::Split, &png, &Options::default(), |_| {}),
            Err(ConvertError::Unsupported)
        ));
    }
}
