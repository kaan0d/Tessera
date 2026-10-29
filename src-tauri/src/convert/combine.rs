//! Merging several files into one output: images, PDFs and office documents into one PDF,
//! audio into one FLAC (WAV when the result would be too big for the in-memory FLAC encoder).
//! The output is named `merged.<ext>` in the first file's folder; originals are never replaced.

use std::path::{Path, PathBuf};

use super::output::OutDir;
use super::{
    audio, image, output, pdf, pdf_edit, security, Conflict, ConvertError, Format, Options,
};

pub const MAX_FILES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MergeKind {
    /// Images, PDFs and office documents into one PDF.
    Pdf,
    /// Audio files into one file.
    Audio,
}

impl MergeKind {
    pub fn arg(self) -> &'static str {
        match self {
            MergeKind::Pdf => "pdf",
            MergeKind::Audio => "audio",
        }
    }

    pub fn from_arg(s: &str) -> Option<MergeKind> {
        match s {
            "pdf" => Some(MergeKind::Pdf),
            "audio" => Some(MergeKind::Audio),
            _ => None,
        }
    }

    /// Merge menu verb for a source extension, if any.
    pub fn for_ext(ext: &str) -> Option<MergeKind> {
        let f = Format::from_ext(ext)?;
        if is_merge_image(f) || is_merge_document(f) {
            Some(MergeKind::Pdf)
        } else if f.kind() == "audio" {
            Some(MergeKind::Audio)
        } else {
            None
        }
    }
}

/// Images the pure-Rust loader reads (no HEIC/AVIF, which need ffmpeg).
fn is_merge_image(f: Format) -> bool {
    matches!(
        f,
        Format::Png
            | Format::Jpg
            | Format::Webp
            | Format::Bmp
            | Format::Gif
            | Format::Tiff
            | Format::Ico
            | Format::Svg
            | Format::Raw
    )
}

/// PDFs and office documents (those need LibreOffice) join images in one PDF.
pub fn is_merge_document(f: Format) -> bool {
    f == Format::Pdf || f.is_office()
}

/// Output location rules: next to the first file (or the fixed folder), never replacing.
fn plan(first: &Path, ext: &str, opts: &Options) -> Result<PathBuf, ConvertError> {
    let mut p = opts.output.clone();
    // a merge never replaces anything, not even an input that is already called merged.*
    p.conflict = Conflict::AutoRename;
    let base = first
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("merged");
    output::plan(&base, ext, &p)
}

fn check(files: &[PathBuf]) -> Result<Vec<(PathBuf, Format)>, ConvertError> {
    if files.is_empty() {
        return Err(ConvertError::NotFound);
    }
    if files.len() > MAX_FILES {
        return Err(ConvertError::TooLarge);
    }
    files.iter().map(|f| security::validate_input(f)).collect()
}

pub fn images_to_pdf(
    files: &[PathBuf],
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let opts = &opts.clamped();
    let list = check(files)?;
    if list.iter().any(|(_, f)| !is_merge_image(*f)) {
        return Err(ConvertError::Unsupported);
    }
    opts.check_cancel()?;
    let dest = plan(&list[0].0, "pdf", opts)?;
    progress(0.0);
    // the pages for `o`, as the bytes of the finished PDF
    let build = |o: &Options,
                 lossy: bool,
                 progress: &mut dyn FnMut(f32)|
     -> Result<Vec<u8>, ConvertError> {
        let (mut pages, mut bytes) = (Vec::new(), 0u64);
        for (i, (src, from)) in list.iter().enumerate() {
            o.check_cancel()?;
            let page = if lossy {
                image::pdf_page_lossy(src, *from, o)?
            } else {
                image::pdf_page(src, *from, o)?
            };
            bytes += page.bytes();
            if bytes > super::transform::MAX_ALLOC {
                return Err(ConvertError::TooLarge);
            }
            pages.push(page);
            progress((i + 1) as f32 / list.len() as f32 * 0.9);
        }
        let payloads: Vec<_> = pages.iter().map(image::Page::payload).collect();
        let mut out = Vec::new();
        pdf::write_pages(&payloads, &mut out)?;
        Ok(out)
    };
    let mut plain = opts.clone();
    plain.max_bytes = None;
    let mut result = build(&plain, false, &mut progress)?;
    if let Some(max) = opts.max_bytes.filter(|m| result.len() as u64 > *m) {
        // over the limit: JPEG pages, quality first, then smaller pictures
        let mut fitted = None;
        'search: for edge in [
            None,
            Some(2400),
            Some(1800),
            Some(1400),
            Some(1000),
            Some(700),
        ] {
            for q in [80u8, 60, 45] {
                let mut o = plain.clone();
                o.resize = edge.map(super::Resize::MaxEdge);
                o.quality.jpeg = q;
                let bytes = build(&o, true, &mut |_| {})?;
                if bytes.len() as u64 <= max {
                    fitted = Some(bytes);
                    break 'search;
                }
            }
        }
        result = fitted.ok_or(ConvertError::TargetSize)?;
    }
    output::write_atomic(&dest, |w| {
        std::io::Write::write_all(w, &result)?;
        Ok(())
    })?;
    progress(1.0);
    Ok(dest)
}

pub fn audio(
    files: &[PathBuf],
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let opts = &opts.clamped();
    let list = check(files)?;
    if list.iter().any(|(_, f)| f.kind() != "audio") {
        return Err(ConvertError::Unsupported);
    }
    let srcs: Vec<(&Path, &str)> = list
        .iter()
        .map(|(p, _)| {
            (
                p.as_path(),
                p.extension().and_then(|e| e.to_str()).unwrap_or(""),
            )
        })
        .collect();
    progress(0.0);
    let mut write = |target: Format| -> Result<PathBuf, ConvertError> {
        let dest = plan(&list[0].0, target.ext(), opts)?;
        output::write_atomic(&dest, |w| {
            audio::merge(&srcs, target, w, &mut progress, opts.cancel.as_deref())
        })?;
        Ok(dest)
    };
    let dest = match write(Format::Flac) {
        Err(ConvertError::TooLarge) => write(Format::Wav)?,
        r => r?,
    };
    progress(1.0);
    Ok(dest)
}

/// Images, PDFs and documents into one PDF, in the given order. Documents go through
/// LibreOffice into a private folder first. A size limit does not apply here.
pub fn files_to_pdf(
    files: &[PathBuf],
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let opts = &opts.clamped();
    let list = check(files)?;
    if list.iter().all(|(_, f)| is_merge_image(*f)) {
        return images_to_pdf(files, opts, progress);
    }
    if list
        .iter()
        .any(|(_, f)| !is_merge_image(*f) && !is_merge_document(*f))
    {
        return Err(ConvertError::Unsupported);
    }
    let dest = plan(&list[0].0, "pdf", opts)?;
    let scratch = super::office::Scratch::new()?;
    let mut inner = opts.clone();
    inner.output = output::OutputParams {
        dir: OutDir::Fixed(scratch.0.clone()),
        ..Default::default()
    };
    inner.max_bytes = None;
    progress(0.0);
    let n = list.len() as f32;
    let mut docs = Vec::with_capacity(list.len());
    let mut held = 0u64;
    for (i, (src, from)) in list.iter().enumerate() {
        opts.check_cancel()?;
        let mut one = || -> Result<(lopdf::Document, u64), ConvertError> {
            Ok(if *from == Format::Pdf {
                (pdf_edit::load(src)?, std::fs::metadata(src)?.len())
            } else if from.is_office() {
                let step = |p: f32| progress((i as f32 + p) / n * 0.9);
                let made = crate::office::convert(src, Format::Pdf, &inner, step)?;
                (pdf_edit::load(&made)?, std::fs::metadata(&made)?.len())
            } else {
                let page = image::pdf_page(src, *from, &inner)?;
                let mut bytes = Vec::new();
                pdf::write_pages(&[page.payload()], &mut bytes)?;
                (pdf_edit::load_mem(&bytes)?, bytes.len() as u64)
            })
        };
        // the error names the file, so a bad one in a long list can be found
        let name = src
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let (doc, bytes) = match one() {
            Err(ConvertError::Cancelled) => return Err(ConvertError::Cancelled),
            Err(e) => return Err(ConvertError::InFile(name, Box::new(e))),
            Ok(x) => x,
        };
        held += bytes;
        if held > super::transform::MAX_ALLOC {
            return Err(ConvertError::TooLarge);
        }
        docs.push(doc);
        progress((i + 1) as f32 / n * 0.9);
    }
    opts.check_cancel()?;
    let result = pdf_edit::save(&mut pdf_edit::join(docs))?;
    output::write_atomic(&dest, |w| {
        std::io::Write::write_all(w, &result)?;
        Ok(())
    })?;
    progress(1.0);
    Ok(dest)
}

/// Runs the merge for `kind`.
pub fn run(
    kind: MergeKind,
    files: &[PathBuf],
    opts: &Options,
    progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    match kind {
        MergeKind::Pdf => files_to_pdf(files, opts, progress),
        MergeKind::Audio => audio(files, opts, progress),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(MergeKind::for_ext("PNG"), Some(MergeKind::Pdf));
        assert_eq!(MergeKind::for_ext("cr2"), Some(MergeKind::Pdf));
        assert_eq!(MergeKind::for_ext("m4a"), Some(MergeKind::Audio));
        assert_eq!(MergeKind::for_ext("pdf"), Some(MergeKind::Pdf));
        assert_eq!(MergeKind::for_ext("docx"), Some(MergeKind::Pdf));
        for e in ["heic", "avif", "mp4", "zzz"] {
            assert_eq!(MergeKind::for_ext(e), None, "{e}");
        }
        assert_eq!(MergeKind::from_arg("audio"), Some(MergeKind::Audio));
        assert_eq!(MergeKind::from_arg("x"), None);
    }

    #[test]
    fn images_become_one_pdf_and_audio_one_flac() {
        let d = tempfile::tempdir().unwrap();
        let mut files = vec![];
        for i in 0..3u8 {
            let p = d.path().join(format!("p{i}.png"));
            ::image::RgbImage::from_pixel(8 + u32::from(i), 8, ::image::Rgb([i * 50, 0, 9]))
                .save(&p)
                .unwrap();
            files.push(p);
        }
        let out = images_to_pdf(&files, &Options::default(), |_| {}).unwrap();
        assert!(out.ends_with("merged.pdf"));
        assert_eq!(pdf::page_count(&out).unwrap(), 3);
        // a second run auto-renames
        let out2 = images_to_pdf(&files, &Options::default(), |_| {}).unwrap();
        assert!(out2.ends_with("merged (1).pdf"));

        let mut wavs = vec![];
        for (i, rate) in [8000u32, 16000].into_iter().enumerate() {
            let p = d.path().join(format!("a{i}.wav"));
            let spec = hound::WavSpec {
                channels: 1 + i as u16,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut w = hound::WavWriter::create(&p, spec).unwrap();
            for n in 0..rate * 2 * (1 + i as u32) {
                w.write_sample((n % 100) as i16).unwrap();
            }
            w.finalize().unwrap();
            wavs.push(p);
        }
        let out = audio(&wavs, &Options::default(), |_| {}).unwrap();
        assert!(out.ends_with("merged.flac"));
        assert!(std::fs::metadata(out).unwrap().len() > 100);
    }

    #[test]
    fn images_and_pdfs_become_one_pdf() {
        let d = tempfile::tempdir().unwrap();
        let png = d.path().join("a.png");
        ::image::RgbImage::new(4, 4).save(&png).unwrap();
        let doc = d.path().join("b.pdf");
        std::fs::write(&doc, pdf_edit::tests::sample(3)).unwrap();
        let out = files_to_pdf(&[doc.clone(), png, doc], &Options::default(), |_| {}).unwrap();
        assert!(out.ends_with("merged.pdf"));
        assert_eq!(pdf::page_count(&out).unwrap(), 7);
    }

    #[test]
    fn mixed_kinds_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.png");
        ::image::RgbImage::new(2, 2).save(&p).unwrap();
        assert!(matches!(
            audio(&[p], &Options::default(), |_| {}),
            Err(ConvertError::Unsupported)
        ));
        assert!(matches!(
            images_to_pdf(&[], &Options::default(), |_| {}),
            Err(ConvertError::NotFound)
        ));
    }
}
