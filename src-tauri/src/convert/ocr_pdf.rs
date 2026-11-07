//! Makes a scanned PDF searchable: every page without a text layer is drawn, read by OCR, and
//! the words are added back as invisible text (render mode 3) placed over the picture, so
//! selecting, searching and copying work while the page looks the same.
//!
//! The text uses one composite font (Identity-H, not embedded: invisible text needs no glyph
//! shapes). Each distinct character gets its own 2-byte code and a `ToUnicode` entry, so any
//! script is stored exactly. Rotated pages get a rotated text matrix.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

use super::ocr::{self, OcrPage};
use super::{ConvertError, Options, pdf, pdf_edit};

const FONT_NAME: &[u8] = b"FTessOcr";
/// Helvetica advance widths of U+0020..=U+007E, in 1/1000 em.
const WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
/// Most distinct characters one document can hold (2-byte codes, 0 unused).
const MAX_CODES: usize = 65_000;

/// Pages the tool looked at.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Pages that got a text layer.
    pub added: usize,
    /// Pages that already had text and were left alone.
    pub had_text: usize,
    /// Pages OCR found no text on, or whose page box cannot be read.
    pub skipped: usize,
}

/// Character -> code table of the layer font, shared by all pages of a document.
#[derive(Default)]
struct Codes {
    map: HashMap<char, u16>,
    order: Vec<char>,
}

impl Codes {
    fn code(&mut self, c: char) -> u16 {
        if let Some(&n) = self.map.get(&c) {
            return n;
        }
        // a full table maps the rest to '?', which is always present first
        if self.order.len() >= MAX_CODES {
            return self.code('?');
        }
        let n = self.order.len() as u16 + 1;
        self.map.insert(c, n);
        self.order.push(c);
        n
    }

    /// The word as 2-byte codes, and its natural width in 1/1000 em.
    fn encode(&mut self, word: &str) -> (Vec<u8>, u32) {
        let (mut bytes, mut width) = (vec![], 0u32);
        for c in word.chars() {
            bytes.extend(self.code(c).to_be_bytes());
            width += advance(c);
        }
        (bytes, width)
    }
}

/// Estimated advance of `c` in 1/1000 em: Helvetica for ASCII, full width for CJK.
fn advance(c: char) -> u32 {
    match c {
        ' '..='~' => u32::from(WIDTHS[c as usize - 32]),
        '\u{2e80}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}' | '\u{f900}'..='\u{faff}'
        | '\u{ff00}'..='\u{ffef}' => 1000,
        _ => 556,
    }
}

fn utf16_hex(c: char) -> String {
    let mut buf = [0u16; 2];
    c.encode_utf16(&mut buf)
        .iter()
        .fold(String::new(), |mut s, u| {
            let _ = write!(s, "{u:04X}");
            s
        })
}

/// The `ToUnicode` CMap: code -> character, in blocks of at most 100 as the format requires.
fn to_unicode(codes: &Codes) -> Vec<u8> {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for (block, chars) in codes.order.chunks(100).enumerate() {
        let _ = writeln!(s, "{} beginbfchar", chars.len());
        for (i, c) in chars.iter().enumerate() {
            let _ = writeln!(s, "<{:04X}> <{}>", block * 100 + i + 1, utf16_hex(*c));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s.into_bytes()
}

/// Fills the reserved font object: Type0 + a non-embedded CID font with the widths.
fn finish_font(doc: &mut Document, font: ObjectId, codes: &Codes) {
    let cmap = doc.add_object(Stream::new(dictionary! {}, to_unicode(codes)));
    let mut widths: Vec<Object> = vec![];
    for (i, c) in codes.order.iter().enumerate() {
        widths.push(Object::Integer(i as i64 + 1));
        widths.push(Object::Array(vec![Object::Integer(i64::from(advance(*c)))]));
    }
    let cid = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => "Arial",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "DW" => 556,
        "W" => widths,
        "CIDToGIDMap" => "Identity",
    });
    doc.objects.insert(
        font,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "Arial",
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(cid)],
            "ToUnicode" => cmap,
        }),
    );
}

fn number(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

/// Where a page's picture lives in PDF space: crop box (else media box) as origin and size of
/// the unrotated page, and the page's clockwise turn (0, 90, 180 or 270).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub turn: u16,
}

fn page_box(page: &Dictionary) -> Option<PageBox> {
    let arr = page
        .get(b"CropBox")
        .or_else(|_| page.get(b"MediaBox"))
        .ok()?
        .as_array()
        .ok()?;
    let v: Vec<f32> = arr.iter().filter_map(number).collect();
    let [a, b, c, d] = v[..] else { return None };
    let (x, y, w, h) = (a.min(c), b.min(d), (c - a).abs(), (d - b).abs());
    if !(w >= 1.0 && h >= 1.0 && w.is_finite() && h.is_finite() && x.is_finite() && y.is_finite()) {
        return None;
    }
    let turn = page
        .get(b"Rotate")
        .ok()
        .and_then(number)
        .map_or(0, |r| (r as i64).rem_euclid(360) as u16);
    // only quarter turns are valid; anything else is drawn upright by readers
    Some(PageBox {
        x,
        y,
        w,
        h,
        turn: if turn.is_multiple_of(90) { turn } else { 0 },
    })
}

/// The page's text as drawing operations: one invisible string per word, stretched to the
/// width OCR measured for it. `pic` is the size in pixels of the picture OCR read (the page as
/// displayed, turn applied).
fn layer_ops(ocr: &OcrPage, pic: (u32, u32), pb: PageBox, codes: &mut Codes) -> Vec<Operation> {
    // displayed page size in points
    let (dw, dh) = if pb.turn.is_multiple_of(180) { (pb.w, pb.h) } else { (pb.h, pb.w) };
    let scale = ocr.scale.max(f32::EPSILON);
    let (kx, ky) = (dw / pic.0 as f32 / scale, dh / pic.1 as f32 / scale);
    let mut ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tr", vec![3.into()]),
    ];
    for word in ocr.lines.iter().flat_map(|l| &l.words) {
        let (bytes, natural) = codes.encode(&word.text);
        if bytes.is_empty() || word.w <= 0.0 || word.h <= 0.0 {
            continue;
        }
        let size = (word.h * ky * 0.9).max(2.0);
        let box_w = word.w * kx;
        // 1/1000 em -> pt at `size`; Tz is a percentage of that width
        let wanted = (box_w / (natural as f32 / 1000.0 * size) * 100.0).clamp(10.0, 1000.0);
        // baseline start in displayed points (origin bottom-left of the displayed page)
        let u = word.x * kx;
        let v = dh - (word.y + word.h) * ky + word.h * ky * 0.2;
        // displayed -> PDF space; (a, b) is the reading direction, (c, d) the up direction
        let (a, b, c, d, x, y) = match pb.turn {
            90 => (0.0, 1.0, -1.0, 0.0, pb.w - v, u),
            180 => (-1.0, 0.0, 0.0, -1.0, pb.w - u, pb.h - v),
            270 => (0.0, -1.0, 1.0, 0.0, v, pb.h - u),
            _ => (1.0, 0.0, 0.0, 1.0, u, v),
        };
        ops.push(Operation::new(
            "Tf",
            vec![Object::Name(FONT_NAME.to_vec()), size.into()],
        ));
        ops.push(Operation::new("Tz", vec![wanted.into()]));
        ops.push(Operation::new(
            "Tm",
            vec![
                a.into(),
                b.into(),
                c.into(),
                d.into(),
                (pb.x + x).into(),
                (pb.y + y).into(),
            ],
        ));
        ops.push(Operation::new(
            "Tj",
            vec![Object::String(bytes, StringFormat::Hexadecimal)],
        ));
        // a space after the word keeps words apart when text is copied
        let (space, _) = codes.encode(" ");
        ops.push(Operation::new(
            "Tj",
            vec![Object::String(space, StringFormat::Hexadecimal)],
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

/// The page box once the page's inherited attributes sit on the page itself.
fn layer_box(doc: &mut Document, id: ObjectId) -> Option<PageBox> {
    pdf_edit::flatten_inherited(doc, id);
    page_box(doc.get_dictionary(id).ok()?)
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
    let mut codes = Codes::default();
    codes.code('?');
    let font = doc.new_object_id();
    for (i, &(number, id)) in pages.iter().enumerate() {
        opts.check_cancel()?;
        if ocr::has_text_layer(doc, number)? {
            out.had_text += 1;
        } else if let Some(pb) = layer_box(doc, id) {
            let img = pdf::render_page(src, number, None)?;
            opts.check_cancel()?;
            let page = ocr::recognize(&img, lang, opts)?;
            if page.lines.iter().all(|l| l.words.is_empty()) {
                out.skipped += 1;
            } else {
                let ops = layer_ops(&page, (img.width(), img.height()), pb, &mut codes);
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
    if out.added > 0 {
        finish_font(doc, font, &codes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::ocr::{Line, Word};
    use super::*;

    fn one_word_page() -> OcrPage {
        OcrPage {
            lines: vec![Line {
                text: "Hi".into(),
                words: vec![Word {
                    text: "Hi".into(),
                    x: 100.0,
                    y: 50.0,
                    w: 40.0,
                    h: 20.0,
                }],
            }],
            scale: 1.0,
        }
    }

    fn tm(ops: &[Operation]) -> Vec<f32> {
        let op = ops.iter().find(|o| o.operator == "Tm").unwrap();
        op.operands.iter().map(|o| number(o).unwrap()).collect()
    }

    #[test]
    fn any_script_gets_codes_and_unicode_entries() {
        let mut c = Codes::default();
        let (a, wa) = c.encode("Aб日");
        assert_eq!(a, vec![0, 1, 0, 2, 0, 3]);
        assert_eq!(wa, 667 + 556 + 1000);
        // the same character keeps its code
        assert_eq!(c.encode("б").0, vec![0, 2]);
        let map = String::from_utf8(to_unicode(&c)).unwrap();
        assert!(map.contains("<0001> <0041>") && map.contains("<0002> <0431>"), "{map}");
        assert!(map.contains("<0003> <65E5>"), "{map}");
        // a character outside the BMP is a surrogate pair
        c.code('😀');
        assert!(String::from_utf8(to_unicode(&c)).unwrap().contains("<D83DDE00>"));
    }

    #[test]
    fn page_box_prefers_crop_orders_corners_and_reads_turn() {
        let d = dictionary! {
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "CropBox" => vec![Object::Real(10.0), 20.into(), 110.into(), 220.into()],
            "Rotate" => 450,
        };
        let pb = page_box(&d).unwrap();
        assert_eq!((pb.x, pb.y, pb.w, pb.h, pb.turn), (10.0, 20.0, 100.0, 200.0, 90));
        assert_eq!(page_box(&dictionary! {}), None);
        let empty = dictionary! { "MediaBox" => vec![0.into(), 0.into(), 0.into(), 0.into()] };
        assert_eq!(page_box(&empty), None);
    }

    #[test]
    fn upright_page_scales_boxes_to_points() {
        // 1000 x 500 px picture on a 500 x 250 pt page: 0.5 pt per px
        let pb = PageBox { x: 0.0, y: 0.0, w: 500.0, h: 250.0, turn: 0 };
        let m = tm(&layer_ops(&one_word_page(), (1000, 500), pb, &mut Codes::default()));
        assert_eq!(&m[..4], &[1.0, 0.0, 0.0, 1.0]);
        assert!((m[4] - 50.0).abs() < 0.01, "{m:?}");
        // word bottom is 70 px = 35 pt from the top; the baseline sits just above 250 - 35
        assert!(m[5] > 215.0 && m[5] < 220.0, "{m:?}");
    }

    #[test]
    fn turned_pages_get_a_turned_text_matrix() {
        // 500 x 250 pt unrotated page shown turned: 0.5 pt per px in every case
        let words = |turn| {
            let pb = PageBox { x: 0.0, y: 0.0, w: 500.0, h: 250.0, turn };
            let (pw, ph) = if turn.is_multiple_of(180) { (1000, 500) } else { (500, 1000) };
            tm(&layer_ops(&one_word_page(), (pw, ph), pb, &mut Codes::default()))
        };
        let m90 = words(90);
        assert_eq!(&m90[..4], &[0.0, 1.0, -1.0, 0.0]);
        assert!((m90[5] - 50.0).abs() < 0.01, "y = u: {m90:?}");
        let m180 = words(180);
        assert_eq!(&m180[..4], &[-1.0, 0.0, 0.0, -1.0]);
        assert!((m180[4] - 450.0).abs() < 0.01, "x = W - u: {m180:?}");
        let m270 = words(270);
        assert_eq!(&m270[..4], &[0.0, -1.0, 1.0, 0.0]);
        assert!((m270[5] - 200.0).abs() < 0.01, "y = H - u: {m270:?}");
    }

    fn scanned_pdf(dir: &Path, turn: Option<i64>) -> std::path::PathBuf {
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
        // a scan stored sideways and turned upright by /Rotate, as scanners write them
        let img = if turn == Some(90) { img.rotate270() } else { img };
        let mut bytes = vec![];
        pdf::write(&pdf::Payload::Pixels(&img), &mut bytes).unwrap();
        let file = dir.join(format!("scan{}.pdf", turn.unwrap_or(0)));
        std::fs::write(&file, bytes).unwrap();
        if let Some(r) = turn {
            let mut doc = pdf_edit::load(&file).unwrap();
            let page = pdf_edit::pages(&doc)[0];
            pdf_edit::rotate(&mut doc, page, r);
            std::fs::write(&file, pdf_edit::save(&mut doc).unwrap()).unwrap();
        }
        file
    }

    #[test]
    fn scanned_and_turned_pdfs_become_searchable() {
        use super::super::pdf_tools::{PdfTool, run};
        use super::super::{Format, ocr as engine};
        if engine::installed_languages().is_empty() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("tessera-ocrpdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for turn in [None, Some(90)] {
            let scan = scanned_pdf(&dir, turn);
            let out = run(&PdfTool::Ocr { lang: None }, &scan, &Options::default(), |_| {}).unwrap();
            let doc = pdf_edit::load(&out).unwrap();
            assert!(engine::has_text_layer(&doc, 1).unwrap(), "{turn:?}");
            // the layer reads back without OCR (the language below would fail it)
            let opts = Options { ocr_lang: Some("xx".into()), ..Options::default() };
            let mut txt = vec![];
            engine::write_text(&out, Format::Pdf, &opts, &mut |_| {}, &mut txt).unwrap();
            let txt = String::from_utf8(txt).unwrap();
            assert!(txt.contains("Scanned") && txt.contains("2026"), "{turn:?} {txt:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
