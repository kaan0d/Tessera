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

use super::ConvertError;

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
}
