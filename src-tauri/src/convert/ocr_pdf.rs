//! Makes a scanned PDF searchable: every page without a text layer is drawn, read by OCR, and
//! the words are added back as invisible text (render mode 3) placed over the picture, so
//! selecting, searching and copying work while the page looks the same.
//!
//! The text uses the standard Helvetica font. Its encoding is WinAnsi plus six codes
//! (1..=6) that name the Turkish letters, so Latin-1 and Turkish text is exact; any other
//! character is stored as `?` (shortcut: no embedded font, add one when Cyrillic or CJK
//! search is wanted).

use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat, dictionary};

use super::ocr::{self, OcrPage};
use super::{ConvertError, Options, pdf, pdf_edit};

const FONT_NAME: &[u8] = b"FTessOcr";
/// Turkish letters outside WinAnsi, stored under codes 1..=6 (see `encode`).
const TURKISH: [(char, &str, u16); 6] = [
    ('Ğ', "Gbreve", 778),
    ('ğ', "gbreve", 556),
    ('Ş', "Scedilla", 667),
    ('ş', "scedilla", 500),
    ('İ', "Idotaccent", 278),
    ('ı', "dotlessi", 222),
];
/// Helvetica advance widths of U+0020..=U+007E, in 1/1000 em.
const WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Pages the tool looked at.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Pages that got a text layer.
    pub added: usize,
    /// Pages that already had text and were left alone.
    pub had_text: usize,
    /// Pages OCR found no text on, or that cannot take a layer (rotated, odd page box).
    pub skipped: usize,
}

/// One word as bytes of the layer font, and its natural width in 1/1000 em.
fn encode(word: &str) -> (Vec<u8>, u32) {
    let (mut bytes, mut width) = (vec![], 0u32);
    for c in word.chars() {
        let (b, w) = if let Some(i) = TURKISH.iter().position(|t| t.0 == c) {
            (i as u8 + 1, u32::from(TURKISH[i].2))
        } else if (' '..='~').contains(&c) {
            (c as u8, u32::from(WIDTHS[c as usize - 32]))
        } else if ('\u{a0}'..='\u{ff}').contains(&c) {
            (c as u32 as u8, 556)
        } else {
            (b'?', 556)
        };
        bytes.push(b);
        width += w;
    }
    (bytes, width)
}

fn font_object() -> Dictionary {
    let mut diff = vec![Object::Integer(1)];
    diff.extend(TURKISH.iter().map(|t| Object::Name(t.1.as_bytes().to_vec())));
    dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => dictionary! {
            "Type" => "Encoding",
            "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => diff,
        },
    }
}

fn number(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

/// `[x0 y0 x1 y1]` of the page's crop box (else media box) as (x, y, width, height).
fn page_box(page: &Dictionary) -> Option<(f32, f32, f32, f32)> {
    let arr = page
        .get(b"CropBox")
        .or_else(|_| page.get(b"MediaBox"))
        .ok()?
        .as_array()
        .ok()?;
    let v: Vec<f32> = arr.iter().filter_map(number).collect();
    let [a, b, c, d] = v[..] else { return None };
    let (x, y, w, h) = (a.min(c), b.min(d), (c - a).abs(), (d - b).abs());
    (w >= 1.0 && h >= 1.0 && w.is_finite() && h.is_finite()).then_some((x, y, w, h))
}

/// The page's text as drawing operations: one invisible string per word, stretched to the
/// width OCR measured for it.
fn layer_ops(ocr: &OcrPage, pic: (u32, u32), pbox: (f32, f32, f32, f32)) -> Vec<Operation> {
    let (x0, y0, w, h) = pbox;
    let (kx, ky) = (
        w / pic.0 as f32 / ocr.scale.max(f32::EPSILON),
        h / pic.1 as f32 / ocr.scale.max(f32::EPSILON),
    );
    let mut ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tr", vec![3.into()]),
    ];
    for word in ocr.lines.iter().flat_map(|l| &l.words) {
        let (mut bytes, natural) = encode(&word.text);
        if bytes.is_empty() || word.w <= 0.0 || word.h <= 0.0 {
            continue;
        }
        let size = (word.h * ky * 0.9).max(2.0);
        let box_w = word.w * kx;
        // 1/1000 em -> pt at `size`; Tz is a percentage of that width
        let wanted = (box_w / (natural as f32 / 1000.0 * size) * 100.0).clamp(10.0, 1000.0);
        let x = x0 + word.x * kx;
        let y = y0 + h - (word.y + word.h) * ky + word.h * ky * 0.2;
        // the trailing space keeps words apart when text is copied
        bytes.push(b' ');
        ops.push(Operation::new(
            "Tf",
            vec![Object::Name(FONT_NAME.to_vec()), size.into()],
        ));
        ops.push(Operation::new("Tz", vec![wanted.into()]));
        ops.push(Operation::new(
            "Tm",
            vec![1.into(), 0.into(), 0.into(), 1.into(), x.into(), y.into()],
        ));
        ops.push(Operation::new(
            "Tj",
            vec![Object::String(bytes, StringFormat::Literal)],
        ));
    }
    ops.push(Operation::new("ET", vec![]));
    ops
}

/// Points `page` at a resource dictionary that also knows the layer font.
fn add_font_resource(doc: &mut Document, page: ObjectId, font: ObjectId) -> Result<(), ConvertError> {
    let dict = doc.get_dictionary(page).map_err(|_| ConvertError::Corrupt)?;
    let deref = |o: Option<&Object>| -> Dictionary {
        o.and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .cloned()
            .unwrap_or_default()
    };
    let mut res = deref(dict.get(b"Resources").ok());
    let mut fonts = deref(res.get(b"Font").ok());
    fonts.set(FONT_NAME.to_vec(), Object::Reference(font));
    res.set("Font", Object::Dictionary(fonts));
    doc.get_dictionary_mut(page)
        .map_err(|_| ConvertError::Corrupt)?
        .set("Resources", Object::Dictionary(res));
    Ok(())
}

/// Adds the text layer to every page of `doc` (loaded from `src`) that lacks one.
pub fn add_text_layers(
    doc: &mut Document,
    src: &Path,
    lang: Option<&str>,
    opts: &Options,
    progress: &mut impl FnMut(f32),
) -> Result<Outcome, ConvertError> {
    let pages: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    if pages.len() > ocr::MAX_PAGES {
        return Err(ConvertError::TooLarge);
    }
    let mut out = Outcome::default();
    let mut font = None;
    for (i, &(number, id)) in pages.iter().enumerate() {
        opts.check_cancel()?;
        if ocr::has_text_layer(doc, number)? {
            out.had_text += 1;
        } else if let Some(pbox) = layer_box(doc, id)
        {
            let img = pdf::render_page(src, number, None)?;
            opts.check_cancel()?;
            let page = ocr::recognize(&img, lang)?;
            if page.lines.iter().all(|l| l.words.is_empty()) {
                out.skipped += 1;
            } else {
                let ops = layer_ops(&page, (img.width(), img.height()), pbox);
                let font = *font.get_or_insert_with(|| doc.add_object(font_object()));
                let bytes = Content { operations: ops }
                    .encode()
                    .map_err(|_| ConvertError::Corrupt)?;
                doc.add_page_contents(id, bytes)
                    .map_err(|_| ConvertError::Corrupt)?;
                add_font_resource(doc, id, font)?;
                out.added += 1;
            }
        } else {
            out.skipped += 1;
        }
        progress((i + 1) as f32 / pages.len() as f32 * 0.95);
    }
    Ok(out)
}

/// The page box when the page can take a layer: unrotated (a turned page would need the
/// text turned too; shortcut: such pages are skipped).
fn layer_box(doc: &mut Document, id: ObjectId) -> Option<(f32, f32, f32, f32)> {
    pdf_edit::flatten_inherited(doc, id);
    let page = doc.get_dictionary(id).ok()?;
    let turned = page
        .get(b"Rotate")
        .ok()
        .and_then(number)
        .is_some_and(|r| (r as i64) % 360 != 0);
    if turned { None } else { page_box(page) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_latin_and_turkish() {
        assert_eq!(encode("Abc"), (b"Abc".to_vec(), 667 + 556 + 500));
        // ğ ş ı take the private codes 1..=6, é is its Latin-1 byte, 日 becomes ?
        assert_eq!(encode("ğşı").0, vec![2, 4, 6]);
        assert_eq!(encode("é").0, vec![0xe9]);
        assert_eq!(encode("日").0, vec![b'?']);
    }

    #[test]
    fn page_box_prefers_crop_and_orders_corners() {
        let d = dictionary! {
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "CropBox" => vec![Object::Real(10.0), 20.into(), 110.into(), 220.into()],
        };
        assert_eq!(page_box(&d), Some((10.0, 20.0, 100.0, 200.0)));
        assert_eq!(page_box(&dictionary! {}), None);
    }

    #[test]
    fn layer_scales_boxes_to_points() {
        use super::super::ocr::{Line, Word};
        let word = Word {
            text: "Hi".into(),
            x: 100.0,
            y: 50.0,
            w: 40.0,
            h: 20.0,
        };
        let page = OcrPage {
            lines: vec![Line {
                text: "Hi".into(),
                words: vec![word],
            }],
            scale: 1.0,
        };
        // 1000 x 500 px picture on a 500 x 250 pt page: 0.5 pt per px
        let ops = layer_ops(&page, (1000, 500), (0.0, 0.0, 500.0, 250.0));
        let tm = ops.iter().find(|o| o.operator == "Tm").unwrap();
        let (x, y) = (number(&tm.operands[4]).unwrap(), number(&tm.operands[5]).unwrap());
        assert!((x - 50.0).abs() < 0.01, "{x}");
        // bottom of the word is at 70 px from the top = 35 pt, so baseline sits just above 250-35
        assert!(y > 215.0 && y < 220.0, "{y}");
    }

    #[test]
    fn scanned_pdf_becomes_searchable() {
        use super::super::pdf_tools::{PdfTool, run};
        use super::super::{Format, ocr as engine};
        if engine::installed_languages().is_empty() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("tessera-ocrpdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("t.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="260">
<rect width="900" height="260" fill="white"/>
<text x="30" y="100" font-family="Arial" font-size="64" fill="black">Scanned page</text>
<text x="30" y="200" font-family="Arial" font-size="64" fill="black">Invoice 2026</text></svg>"#,
        )
        .unwrap();
        let img = super::super::svg::render(&svg, None).unwrap();
        let mut bytes = vec![];
        pdf::write(&pdf::Payload::Pixels(&img), &mut bytes).unwrap();
        let scan = dir.join("scan.pdf");
        std::fs::write(&scan, bytes).unwrap();

        let out = run(&PdfTool::Ocr { lang: None }, &scan, &Options::default(), |_| {}).unwrap();
        assert!(out.ends_with("scan-searchable.pdf"), "{out:?}");
        // the layer is readable without OCR now, and the picture is still there
        let doc = pdf_edit::load(&out).unwrap();
        assert!(engine::has_text_layer(&doc, 1).unwrap());
        let mut txt = vec![];
        let opts = Options {
            ocr_lang: Some("xx".into()), // would fail if OCR were needed
            ..Options::default()
        };
        engine::write_text(&out, Format::Pdf, &opts, &mut |_| {}, &mut txt).unwrap();
        let txt = String::from_utf8(txt).unwrap();
        let has_image = doc.objects.values().any(|o| {
            o.as_stream().is_ok_and(|s| {
                s.dict.get(b"Subtype").and_then(Object::as_name).is_ok_and(|n| n == b"Image")
            })
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(has_image);
        assert!(txt.contains("Scanned") && txt.contains("2026"), "{txt:?}");
    }
}
