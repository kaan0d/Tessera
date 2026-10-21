//! Camera RAW source (DNG, CR2, CR3, NEF, ARW, ORF, RW2, RAF).
//!
//! No permissive crate demosaics RAW (rawler and rawloader are LGPL-2.1), so this takes
//! the largest full-color JPEG preview that the camera embedded in the file. Every one of
//! these formats stores one; it is what the camera itself shows. The result is the
//! camera's own rendering, not a re-developed sensor image. Files without a usable
//! preview (some DNGs from phones hold only a thumbnail) give the largest thumbnail.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use image::{DynamicImage, ImageFormat, ImageReader};

use super::transform::{self, MAX_ALLOC, MAX_DIM};
use super::ConvertError;

const CHUNK: usize = 1 << 20;
/// JPEG start markers examined per file (sensor data can hold stray FF D8 FF bytes).
const MAX_CANDIDATES: usize = 512;
/// Bytes of an embedded JPEG read to find its EXIF block.
const EXIF_PREFIX: u64 = 128 * 1024;

/// Offsets of every `FF D8 FF` in the file.
fn candidates(f: &mut File) -> std::io::Result<Vec<u64>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; CHUNK + 2];
    let (mut base, mut carry) = (0u64, 0usize);
    loop {
        let n = f.read(&mut buf[carry..])?;
        if n == 0 {
            return Ok(out);
        }
        let end = carry + n;
        let mut i = 0;
        while i + 2 < end {
            if buf[i] == 0xFF && buf[i + 1] == 0xD8 && buf[i + 2] == 0xFF {
                out.push(base + i as u64);
                if out.len() >= MAX_CANDIDATES {
                    return Ok(out);
                }
            }
            i += 1;
        }
        // keep the last two bytes: a marker may straddle the chunk edge
        let keep = end.min(2);
        buf.copy_within(end - keep..end, 0);
        base += (end - keep) as u64;
        carry = keep;
    }
}

fn jpeg_at(path: &Path, off: u64) -> Result<ImageReader<BufReader<File>>, ConvertError> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(off))?;
    Ok(ImageReader::with_format(
        BufReader::new(f),
        ImageFormat::Jpeg,
    ))
}

/// The largest embedded JPEG, decoded, with the EXIF orientation that belongs to it
/// (not yet applied).
pub fn decode(src: &Path) -> Result<(DynamicImage, Option<u8>), ConvertError> {
    let mut f = File::open(src)?;
    let mut best: Option<(u64, u64)> = None; // (pixels, offset)
    for off in candidates(&mut f)? {
        let Ok((w, h)) = jpeg_at(src, off)?.into_dimensions() else {
            continue;
        };
        let px = u64::from(w) * u64::from(h);
        if w <= MAX_DIM && h <= MAX_DIM && px > best.map_or(0, |b| b.0) {
            best = Some((px, off));
        }
    }
    let (_, off) = best.ok_or(ConvertError::Corrupt)?;

    let mut head = Vec::new();
    f.seek(SeekFrom::Start(off))?;
    f.by_ref().take(EXIF_PREFIX).read_to_end(&mut head)?;
    // the preview's own tag wins; otherwise the file's (TIFF-based RAW keeps it in IFD0)
    let orientation =
        transform::bytes_orientation(&head).or_else(|| transform::file_orientation(src));

    let mut rd = jpeg_at(src, off)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIM);
    limits.max_image_height = Some(MAX_DIM);
    limits.max_alloc = Some(MAX_ALLOC);
    rd.limits(limits);
    let img = DynamicImage::from_decoder(rd.into_decoder()?)?;
    Ok((img, orientation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn jpeg(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut v)
            .encode_image(&RgbImage::from_pixel(w, h, Rgb(c)))
            .unwrap();
        v
    }

    /// Fake RAW: junk, a thumbnail, more junk with stray markers, the big preview, junk.
    fn fake_raw(dir: &Path, name: &str) -> std::path::PathBuf {
        let mut b = b"II*\0\x08\0\0\0".to_vec();
        b.extend(vec![0x11; 3000]);
        b.extend(jpeg(16, 8, [255, 0, 0]));
        b.extend([0xFF, 0xD8, 0xFF, 0x00, 0xFF, 0xD8, 0xFF]); // stray, undecodable
        b.extend(vec![0x22; CHUNK + 17]); // forces a chunk-boundary pass
        b.extend(jpeg(96, 64, [0, 200, 0]));
        b.extend(vec![0x33; 500]);
        let p = dir.join(name);
        std::fs::write(&p, b).unwrap();
        p
    }

    #[test]
    fn picks_the_largest_preview() {
        let d = tempfile::tempdir().unwrap();
        let img = decode(&fake_raw(d.path(), "a.nef")).unwrap().0.to_rgb8();
        assert_eq!(img.dimensions(), (96, 64));
        let p = img.get_pixel(40, 30).0;
        assert!(p[1] > 150 && p[0] < 80, "{p:?}");
    }

    #[test]
    fn marker_across_a_chunk_edge_is_found() {
        let d = tempfile::tempdir().unwrap();
        let mut b = vec![0x44u8; CHUNK + 1]; // FF D8 FF starts on the last byte of the first read
        b.extend(jpeg(40, 40, [9, 9, 200]));
        let p = d.path().join("e.dng");
        std::fs::write(&p, b).unwrap();
        assert_eq!(decode(&p).unwrap().0.width(), 40);
    }

    #[test]
    fn no_preview_or_garbage_is_corrupt() {
        let d = tempfile::tempdir().unwrap();
        for (n, bytes) in [
            ("a.cr2", vec![0u8; 5000]),
            ("b.arw", b"\xFF\xD8\xFF".to_vec()),
            ("c.raf", (0..20_000u32).map(|i| (i * 7) as u8).collect()),
            (
                "d.dng",
                [&[0xFF, 0xD8, 0xFF, 0xE0][..], &[0xAB; 600]].concat(),
            ),
        ] {
            let p = d.path().join(n);
            std::fs::write(&p, bytes).unwrap();
            assert!(decode(&p).is_err(), "{n}");
        }
    }
}
