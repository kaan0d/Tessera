//! Text recognition through the Windows OCR engine (`Windows.Media.Ocr`): offline, no download,
//! but only for the languages Windows has a recognizer pack for. Pixels go in as a
//! `SoftwareBitmap`; words come back with boxes in pixels (top-left origin) of the image
//! that was passed in. Callers run this on a worker thread: the async calls are blocked on.

use image::DynamicImage;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::HSTRING;

use std::io::Write;
use std::path::Path;

use super::{ConvertError, Format, Options};

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub words: Vec<Word>,
}

/// Result for one picture. `scale` is how much smaller the recognized picture was than the
/// original (1.0 = same size); multiply word boxes by `1 / scale` to map them back.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OcrPage {
    pub lines: Vec<Line>,
    pub scale: f32,
}

impl OcrPage {
    pub fn text(&self) -> String {
        let lines: Vec<&str> = self.lines.iter().map(|l| l.text.as_str()).collect();
        lines.join("\r\n")
    }
}

/// `tr`, `en-US`, `zh-Hans-CN`: a BCP-47 tag, at most 35 characters.
pub fn valid_lang(tag: &str) -> bool {
    (2..=35).contains(&tag.len())
        && tag.split('-').all(|p| {
            !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        && tag.as_bytes()[0].is_ascii_alphabetic()
}

fn os(e: windows::core::Error) -> ConvertError {
    ConvertError::OcrFailed(e.message())
}

/// Recognizer languages installed in Windows, as BCP-47 tags.
pub fn installed_languages() -> Vec<String> {
    init();
    OcrEngine::AvailableRecognizerLanguages()
        .map(|v| {
            v.into_iter()
                .filter_map(|l| l.LanguageTag().ok().map(|t| t.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// WinRT needs the thread in an apartment. Already initialized (any mode) is fine.
fn init() {
    // SAFETY: plain FFI call with a valid flag; a repeat call just returns S_FALSE.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

fn engine(lang: Option<&str>) -> Result<OcrEngine, ConvertError> {
    if let Some(tag) = lang.filter(|t| valid_lang(t)) {
        let l = Language::CreateLanguage(&HSTRING::from(tag)).map_err(os)?;
        return OcrEngine::TryCreateFromLanguage(&l).map_err(|_| ConvertError::OcrNoLanguage);
    }
    OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| ConvertError::OcrNoLanguage)
}

/// Recognizes the text of `img` in `lang` (BCP-47) or in the user's Windows languages.
pub fn recognize(img: &DynamicImage, lang: Option<&str>) -> Result<OcrPage, ConvertError> {
    init();
    let engine = engine(lang)?;
    let max = OcrEngine::MaxImageDimension().map_err(os)?;
    let (w0, h0) = (img.width(), img.height());
    if w0 == 0 || h0 == 0 {
        return Err(ConvertError::Corrupt);
    }
    let long = w0.max(h0);
    let scale = if long > max { max as f32 / long as f32 } else { 1.0 };
    let small;
    let src = if scale < 1.0 {
        let (w, h) = (
            ((w0 as f32 * scale) as u32).max(1),
            ((h0 as f32 * scale) as u32).max(1),
        );
        small = img.resize_exact(w, h, image::imageops::FilterType::Triangle);
        &small
    } else {
        img
    };
    let (w, h) = (src.width(), src.height());
    // BGRA over white: the engine reads a transparent pixel as black text on nothing
    let rgba = src.to_rgba8();
    let mut bgra = Vec::with_capacity(rgba.len());
    for p in rgba.pixels() {
        let a = p[3] as u32;
        let over = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        bgra.extend([over(p[2]), over(p[1]), over(p[0]), 255]);
    }
    let writer = DataWriter::new().map_err(os)?;
    writer.WriteBytes(&bgra).map_err(os)?;
    let buffer = writer.DetachBuffer().map_err(os)?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w as i32, h as i32)
            .map_err(os)?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(os)?
        .join()
        .map_err(os)?;
    let mut lines = vec![];
    for line in result.Lines().map_err(os)? {
        let mut words = vec![];
        for word in line.Words().map_err(os)? {
            let r = word.BoundingRect().map_err(os)?;
            words.push(Word {
                text: word.Text().map_err(os)?.to_string(),
                x: r.X,
                y: r.Y,
                w: r.Width,
                h: r.Height,
            });
        }
        lines.push(Line {
            text: line.Text().map_err(os)?.to_string(),
            words,
        });
    }
    Ok(OcrPage { lines, scale })
}

/// Pages with fewer readable characters than this are read by OCR instead of from the PDF.
const MIN_EMBEDDED: usize = 10;
/// A PDF with more pages than this is refused (OCR time and memory).
const MAX_PAGES: usize = 500;

/// Picture or PDF file -> UTF-8 text (CRLF lines, pages separated by a blank line). A PDF page
/// with a text layer is read from it; a scanned page goes through OCR. No text at all is
/// `OcrNoText`.
pub fn write_text(
    src: &Path,
    from: Format,
    opts: &Options,
    progress: &mut impl FnMut(f32),
    w: &mut impl Write,
) -> Result<(), ConvertError> {
    let lang = opts.ocr_lang.as_deref();
    let text = if from == Format::Pdf {
        pdf_text(src, lang, opts, progress)?
    } else {
        // size rules are for the output picture, not for what the engine reads
        let load_opts = Options {
            resize: None,
            ..opts.clone()
        };
        let img = super::image::load(src, from, &load_opts)?;
        opts.check_cancel()?;
        recognize(&img, lang)?.text()
    };
    if text.trim().is_empty() {
        return Err(ConvertError::OcrNoText);
    }
    w.write_all(text.as_bytes())?;
    progress(1.0);
    Ok(())
}

fn pdf_text(
    src: &Path,
    lang: Option<&str>,
    opts: &Options,
    progress: &mut impl FnMut(f32),
) -> Result<String, ConvertError> {
    let doc = super::pdf_edit::load(src)?;
    let numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    if numbers.len() > MAX_PAGES {
        return Err(ConvertError::TooLarge);
    }
    let mut pages = vec![];
    for (i, &n) in numbers.iter().enumerate() {
        opts.check_cancel()?;
        let embedded = super::pdf_edit::guarded(|| {
            Ok(doc
                .extract_text_with_limit(&[n], 8 * 1024 * 1024)
                .unwrap_or_default())
        })?;
        let readable = embedded.chars().filter(|c| c.is_alphanumeric()).count();
        let text = if readable >= MIN_EMBEDDED {
            crlf(embedded.trim())
        } else {
            let img = super::pdf::render_page(src, n, None)?;
            opts.check_cancel()?;
            recognize(&img, lang)?.text()
        };
        pages.push(text);
        progress((i + 1) as f32 / numbers.len() as f32 * 0.99);
    }
    Ok(pages.join("\r\n\r\n"))
}

/// Line breaks as CRLF, whatever the source used.
fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags() {
        for ok in ["tr", "en-US", "zh-Hans-CN", "pt-BR"] {
            assert!(valid_lang(ok), "{ok}");
        }
        for bad in ["", "t", "-tr", "tr-", "en--US", "../x", "tr US", "1en", "a".repeat(40).as_str()] {
            assert!(!valid_lang(bad), "{bad}");
        }
    }

    #[test]
    fn blank_picture_has_no_text() {
        if installed_languages().is_empty() {
            return; // no recognizer pack on this machine
        }
        let img = DynamicImage::new_rgb8(200, 100);
        let page = recognize(&img, None).unwrap();
        assert!(page.lines.is_empty());
        assert_eq!(page.scale, 1.0);
    }

    #[test]
    fn oversized_picture_is_scaled_down() {
        if installed_languages().is_empty() {
            return;
        }
        let img = DynamicImage::new_rgb8(12_000, 40);
        let page = recognize(&img, None).unwrap();
        assert!(page.scale < 1.0 && page.scale > 0.0);
    }

    #[test]
    fn reads_rendered_text_into_a_txt_file() {
        if installed_languages().is_empty() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("tessera-ocr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("t.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="260">
<rect width="900" height="260" fill="white"/>
<text x="30" y="100" font-family="Arial" font-size="64" fill="black">Hello Tessera</text>
<text x="30" y="200" font-family="Arial" font-size="64" fill="black">Invoice 2026</text></svg>"#,
        )
        .unwrap();
        let mut out = vec![];
        write_text(&svg, Format::Svg, &Options::default(), &mut |_| {}, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.contains("Hello") && text.contains("2026"), "{text:?}");
        assert!(text.contains("\r\n"));
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tessera-ocr-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const WORDS_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="260">
<rect width="900" height="260" fill="white"/>
<text x="30" y="100" font-family="Arial" font-size="64" fill="black">Scanned page</text>
<text x="30" y="200" font-family="Arial" font-size="64" fill="black">Invoice 2026</text></svg>"#;

    #[test]
    fn scanned_pdf_goes_through_ocr() {
        if installed_languages().is_empty() {
            return;
        }
        let dir = temp_dir("scan");
        let svg = dir.join("t.svg");
        std::fs::write(&svg, WORDS_SVG).unwrap();
        let img = super::super::svg::render(&svg, None).unwrap();
        let mut pdf = vec![];
        super::super::pdf::write(&super::super::pdf::Payload::Pixels(&img), &mut pdf).unwrap();
        let file = dir.join("scan.pdf");
        std::fs::write(&file, pdf).unwrap();
        let mut out = vec![];
        write_text(&file, Format::Pdf, &Options::default(), &mut |_| {}, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.contains("Scanned") && text.contains("2026"), "{text:?}");
    }

    #[test]
    fn pdf_with_text_layer_is_read_without_ocr() {
        use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
        let (catalog, tree, page, font, content) =
            (Ref::new(1), Ref::new(2), Ref::new(3), Ref::new(4), Ref::new(5));
        let mut pdf = Pdf::new();
        pdf.catalog(catalog).pages(tree);
        pdf.pages(tree).kids([page]).count(1);
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 300.0, 200.0))
            .parent(tree)
            .contents(content);
        p.resources().fonts().pair(Name(b"F1"), font);
        p.finish();
        pdf.type1_font(font).base_font(Name(b"Helvetica"));
        let mut c = Content::new();
        c.begin_text()
            .set_font(Name(b"F1"), 18.0)
            .next_line(20.0, 100.0)
            .show(Str(b"Embedded text layer 12345"))
            .end_text();
        pdf.stream(content, &c.finish());
        let dir = temp_dir("layer");
        let file = dir.join("t.pdf");
        std::fs::write(&file, pdf.finish()).unwrap();
        let mut out = vec![];
        // `lang` is bogus on purpose: the OCR engine must not be needed
        let opts = Options {
            ocr_lang: Some("xx".into()),
            ..Options::default()
        };
        write_text(&file, Format::Pdf, &opts, &mut |_| {}, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.contains("Embedded text layer 12345"), "{text:?}");
    }
}
