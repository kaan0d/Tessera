//! Editing existing PDFs with lopdf (pure Rust): joining several into one and picking pages.
//! PDFs are untrusted: the file size and every decompressed stream are capped, and a panic
//! inside the parser becomes `Corrupt`. A PDF that needs a password is `Protected`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use lopdf::{Document, LoadOptions, Object, ObjectId, dictionary};

use super::ConvertError;

const MAX_FILE: u64 = 256 * 1024 * 1024;
/// Per object or cross-reference stream (those are decoded while loading); real files stay far
/// below this, a crafted one pays at most this much per stream.
const MAX_STREAM: usize = 32 * 1024 * 1024;
/// Page attributes a page may inherit from its parents in the page tree.
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

pub(crate) fn guarded<T>(f: impl FnOnce() -> Result<T, ConvertError>) -> Result<T, ConvertError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(Err(ConvertError::Corrupt))
}

pub fn load_mem(bytes: &[u8]) -> Result<Document, ConvertError> {
    guarded(|| {
        let opts = LoadOptions {
            max_decompressed_size: Some(MAX_STREAM),
            ..LoadOptions::default()
        };
        let doc =
            Document::load_mem_with_options(bytes, opts).map_err(|_| ConvertError::Corrupt)?;
        // an empty user password is opened by the loader; anything still encrypted needs one
        if doc.is_encrypted() {
            return Err(ConvertError::Protected);
        }
        // a damaged file (broken or cut-off cross-reference table) is refused, not patched up:
        // the result could silently miss pages
        if doc.xref_recovery.is_some() {
            return Err(ConvertError::Corrupt);
        }
        if doc.get_pages().is_empty() {
            return Err(ConvertError::Corrupt);
        }
        Ok(doc)
    })
}

pub fn load(path: &Path) -> Result<Document, ConvertError> {
    if std::fs::metadata(path)?.len() > MAX_FILE {
        return Err(ConvertError::TooLarge);
    }
    load_mem(&std::fs::read(path)?)
}

/// Compressed streams and object streams; the header says 1.7 since object streams need 1.5.
/// Refuses a PDF that is damaged, locked or empty before another engine reads it.
pub fn check_intact(path: &Path) -> Result<(), ConvertError> {
    load(path).map(|_| ())
}

pub fn save(doc: &mut Document) -> Result<Vec<u8>, ConvertError> {
    guarded(|| {
        doc.compress();
        doc.version = "1.7".into();
        let mut out = Vec::new();
        doc.save_modern(&mut out)
            .map_err(|_| ConvertError::Corrupt)?;
        Ok(out)
    })
}

/// Turns `page` clockwise by `angle` degrees on top of the rotation it has (or inherits).
pub fn rotate(doc: &mut Document, page: ObjectId, angle: i64) {
    flatten_inherited(doc, page);
    if let Ok(d) = doc.get_dictionary_mut(page) {
        let now = d.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
        d.set("Rotate", (now + angle).rem_euclid(360));
    }
}

/// Page ids in reading order.
pub fn pages(doc: &Document) -> Vec<ObjectId> {
    doc.get_pages().into_values().collect()
}

/// Copies what `page` inherits from the page tree onto the page itself, so it can move to a
/// new tree without losing its fonts or size.
fn flatten_inherited(doc: &mut Document, page: ObjectId) {
    let Ok(own) = doc.get_dictionary(page) else {
        return;
    };
    let mut missing: Vec<&[u8]> = INHERITED.into_iter().filter(|k| !own.has(k)).collect();
    let mut found = vec![];
    let mut parent = own.get(b"Parent").and_then(Object::as_reference).ok();
    // a page tree deeper than this is broken or hostile
    for _ in 0..64 {
        let Some(node) = parent.and_then(|id| doc.get_dictionary(id).ok()) else {
            break;
        };
        missing.retain(|k| match node.get(k) {
            Ok(v) => {
                found.push((k.to_vec(), v.clone()));
                false
            }
            Err(_) => true,
        });
        parent = node.get(b"Parent").and_then(Object::as_reference).ok();
    }
    if let Ok(d) = doc.get_dictionary_mut(page) {
        for (k, v) in found {
            d.set(k, v);
        }
    }
}

/// Makes `kids` (in this order) the only pages, under a fresh catalog. Bookmarks, forms and
/// other catalog entries are dropped: they would point at pages that may be gone.
pub fn keep_pages(doc: &mut Document, kids: &[ObjectId]) {
    for &id in kids {
        flatten_inherited(doc, id);
    }
    let tree = doc.new_object_id();
    for &id in kids {
        if let Ok(d) = doc.get_dictionary_mut(id) {
            d.set("Parent", tree);
        }
    }
    let refs: Vec<Object> = kids.iter().map(|&id| id.into()).collect();
    doc.objects.insert(
        tree,
        dictionary! { "Type" => "Pages", "Kids" => refs, "Count" => kids.len() as i64 }.into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree });
    // title and author survive
    let info = doc.trailer.get(b"Info").ok().cloned();
    doc.trailer = dictionary! { "Root" => catalog };
    if let Some(info) = info {
        doc.trailer.set("Info", info);
    }
    doc.prune_objects();
}

/// All pages of `docs`, in order, as one document.
pub fn join(docs: Vec<Document>) -> Document {
    let mut out = Document::with_version("1.7");
    let mut kids = vec![];
    for mut d in docs {
        d.renumber_objects_with(out.max_id + 1);
        let ids = pages(&d);
        for &id in &ids {
            flatten_inherited(&mut d, id);
        }
        kids.extend(ids);
        out.max_id = out.max_id.max(d.max_id);
        out.objects.extend(d.objects);
    }
    keep_pages(&mut out, &kids);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pdf_writer::{Pdf, Rect, Ref};

    /// A PDF of `n` empty pages; page i is 100+i points wide, inheriting its size from the tree.
    pub(crate) fn sample(n: usize) -> Vec<u8> {
        let mut pdf = Pdf::new();
        let (cat, tree) = (Ref::new(1), Ref::new(2));
        pdf.catalog(cat).pages(tree);
        let ids: Vec<Ref> = (0..n).map(|i| Ref::new(3 + i as i32)).collect();
        pdf.pages(tree)
            .kids(ids.iter().copied())
            .count(n as i32)
            .media_box(Rect::new(0.0, 0.0, 50.0, 50.0));
        for (i, id) in ids.iter().enumerate() {
            let mut p = pdf.page(*id);
            p.parent(tree);
            if i > 0 {
                p.media_box(Rect::new(0.0, 0.0, 100.0 + i as f32, 100.0));
            }
        }
        pdf.finish()
    }

    pub(crate) fn widths(doc: &Document) -> Vec<f32> {
        pages(doc)
            .into_iter()
            .map(|id| {
                let b = doc
                    .get_dictionary(id)
                    .unwrap()
                    .get(b"MediaBox")
                    .unwrap()
                    .as_array()
                    .unwrap();
                b[2].as_float().unwrap()
            })
            .collect()
    }

    fn reload(doc: &mut Document) -> Document {
        load_mem(&save(doc).unwrap()).unwrap()
    }

    #[test]
    fn join_keeps_order_and_inherited_sizes() {
        let a = load_mem(&sample(2)).unwrap();
        let b = load_mem(&sample(3)).unwrap();
        let mut j = join(vec![a, b]);
        let j = reload(&mut j);
        assert_eq!(widths(&j), [50.0, 101.0, 50.0, 101.0, 102.0]);
    }

    #[test]
    fn keep_pages_picks_and_reorders() {
        let mut d = load_mem(&sample(4)).unwrap();
        let p = pages(&d);
        keep_pages(&mut d, &[p[3], p[0]]);
        let d = reload(&mut d);
        assert_eq!(widths(&d), [103.0, 50.0]);
    }

    #[test]
    fn junk_and_empty_input_are_errors() {
        assert!(matches!(load_mem(b"not a pdf"), Err(ConvertError::Corrupt)));
        // cut off at two thirds: the cross-reference table is gone
        let whole = sample(3);
        assert!(load_mem(&whole).is_ok());
        let cut = &whole[..whole.len() * 2 / 3];
        assert!(matches!(load_mem(cut), Err(ConvertError::Corrupt)));
        assert!(matches!(load_mem(&sample(0)), Err(ConvertError::Corrupt)));
    }
}
