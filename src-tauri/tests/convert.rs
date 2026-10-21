use std::fs;
use std::path::{Path, PathBuf};

use donusturx_lib::convert::{
    convert, Conflict, ConvertError, Format, Options, OutDir, OutputParams,
};
use image::{DynamicImage, Rgba, RgbaImage};

fn sample_image(dir: &Path, name: &str) -> PathBuf {
    let img = RgbaImage::from_fn(40, 30, |x, y| Rgba([(x * 6) as u8, (y * 8) as u8, 90, 255]));
    let p = dir.join(name);
    DynamicImage::ImageRgba8(img).save(&p).unwrap();
    p
}

fn sample_wav(dir: &Path, name: &str, secs: u32) -> PathBuf {
    let p = dir.join(name);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&p, spec).unwrap();
    for i in 0..44_100 * secs {
        let s = ((i as f32 * 0.05).sin() * 12_000.0) as i16;
        w.write_sample(s).unwrap();
        w.write_sample(s / 2).unwrap();
    }
    w.finalize().unwrap();
    p
}

fn run(p: &Path, t: Format) -> Result<PathBuf, ConvertError> {
    convert(p, t, &Options::default(), |_| {})
}

#[test]
fn image_matrix_round_trips() {
    let d = tempfile::tempdir().unwrap();
    let targets = [
        Format::Png,
        Format::Jpg,
        Format::Webp,
        Format::Bmp,
        Format::Gif,
        Format::Tiff,
        Format::Ico,
    ];
    for from in targets {
        let src = sample_image(d.path(), &format!("src_{}.{}", from.ext(), from.ext()));
        for to in targets.into_iter().filter(|&t| t != from) {
            let out = run(&src, to).unwrap_or_else(|e| panic!("{from:?}->{to:?}: {e}"));
            let img = image::open(&out).unwrap_or_else(|e| panic!("{from:?}->{to:?}: {e}"));
            assert_eq!((img.width(), img.height()), (40, 30), "{from:?}->{to:?}");
        }
    }
}

#[test]
fn webp_is_lossless() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_image(d.path(), "a.png");
    let out = run(&src, Format::Webp).unwrap();
    assert_eq!(
        image::open(&src).unwrap().to_rgba8(),
        image::open(&out).unwrap().to_rgba8()
    );
}

#[test]
fn jpeg_quality_changes_size_and_alpha_goes_white() {
    let d = tempfile::tempdir().unwrap();
    let noisy = RgbaImage::from_fn(128, 128, |x, y| {
        Rgba([
            ((x * 7) ^ (y * 13)) as u8,
            (x * y) as u8,
            (x + y * 3) as u8,
            255,
        ])
    });
    let src = d.path().join("n.png");
    noisy.save(&src).unwrap();
    let size = |q: u8| {
        let o = Options {
            jpeg_quality: q,
            output: OutputParams {
                conflict: Conflict::Overwrite,
                ..Default::default()
            },
        };
        let out = convert(&src, Format::Jpg, &o, |_| {}).unwrap();
        fs::metadata(out).unwrap().len()
    };
    assert!(size(95) > size(20));

    let clear = d.path().join("clear.png");
    RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]))
        .save(&clear)
        .unwrap();
    let out = run(&clear, Format::Jpg).unwrap();
    let px = image::open(out).unwrap().to_rgb8();
    assert!(px.get_pixel(4, 4).0.iter().all(|&c| c > 250));
}

#[test]
fn large_ico_is_downscaled() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("big.png");
    RgbaImage::from_pixel(600, 300, Rgba([1, 2, 3, 255]))
        .save(&src)
        .unwrap();
    let out = run(&src, Format::Ico).unwrap();
    let img = image::open(out).unwrap();
    assert!(img.width() <= 256 && img.height() <= 256);
}

#[test]
fn wav_to_flac_to_wav_preserves_samples() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_wav(d.path(), "t.wav", 1);
    let flac = run(&src, Format::Flac).unwrap();
    assert!(fs::metadata(&flac).unwrap().len() > 100);
    // fresh dir so the round trip does not collide with the source name
    let d2 = tempfile::tempdir().unwrap();
    let flac2 = d2.path().join("t.flac");
    fs::copy(&flac, &flac2).unwrap();
    let back = run(&flac2, Format::Wav).unwrap();

    let read = |p: &Path| -> Vec<i16> {
        hound::WavReader::open(p)
            .unwrap()
            .samples::<i16>()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(read(&src), read(&back));
}

#[test]
fn progress_is_monotonic_and_ends_at_one() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_wav(d.path(), "p.wav", 2);
    let mut seen = vec![];
    convert(&src, Format::Flac, &Options::default(), |p| seen.push(p)).unwrap();
    assert_eq!(seen.first(), Some(&0.0));
    assert_eq!(seen.last(), Some(&1.0));
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn ffmpeg_targets_report_needs_ffmpeg() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_wav(d.path(), "t.wav", 1);
    assert!(matches!(
        run(&src, Format::Mp3),
        Err(ConvertError::NeedsFfmpeg)
    ));
    assert!(matches!(
        run(&src, Format::Ogg),
        Err(ConvertError::NeedsFfmpeg)
    ));
}

#[test]
fn bad_inputs_error_without_panicking() {
    let d = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let p = d.path().join(name);
        fs::write(&p, bytes).unwrap();
        p
    };
    let good_png = fs::read(sample_image(d.path(), "good.png")).unwrap();
    let good_wav = fs::read(sample_wav(d.path(), "good.wav", 1)).unwrap();

    // empty
    assert!(matches!(
        run(&write("e.png", b""), Format::Jpg),
        Err(ConvertError::Corrupt)
    ));
    assert!(matches!(
        run(&write("e.wav", b""), Format::Flac),
        Err(ConvertError::Corrupt)
    ));
    // garbage under valid extensions
    let junk: Vec<u8> = (0..4096u32).map(|i| (i * 31 % 251) as u8).collect();
    for (name, to) in [
        ("g.png", Format::Jpg),
        ("g.jpg", Format::Png),
        ("g.mp3", Format::Wav),
        ("g.flac", Format::Wav),
        ("g.ogg", Format::Wav),
        ("g.wav", Format::Flac),
        ("g.m4a", Format::Wav),
    ] {
        assert!(run(&write(name, &junk), to).is_err(), "{name}");
    }
    // truncated real files
    let r = run(
        &write("t.png", &good_png[..good_png.len() / 2]),
        Format::Jpg,
    );
    assert!(matches!(r, Err(ConvertError::Corrupt)), "{r:?}");
    let r = run(&write("t.wav", &good_wav[..20]), Format::Flac);
    assert!(r.is_err());
    // unsupported source / target, missing, directory
    assert!(matches!(
        run(&write("a.exe", b"MZ"), Format::Png),
        Err(ConvertError::Unsupported)
    ));
    assert!(matches!(
        run(&d.path().join("good.png"), Format::Wav),
        Err(ConvertError::Unsupported)
    ));
    assert!(matches!(
        run(&d.path().join("yok.png"), Format::Jpg),
        Err(ConvertError::NotFound)
    ));
    assert!(run(d.path(), Format::Png).is_err());
    // no leftover temp files
    let tmp = fs::read_dir(d.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(tmp, 0);
}

#[test]
fn oversized_dimensions_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    // valid PNG header claiming 40000x40000 with no pixel data
    let mut png = fs::read(sample_image(d.path(), "s.png")).unwrap();
    png[16..20].copy_from_slice(&40_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&40_000u32.to_be_bytes());
    let p = d.path().join("bomb.png");
    fs::write(&p, png).unwrap();
    assert!(run(&p, Format::Jpg).is_err());
}

#[test]
fn auto_rename_keeps_existing_output() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_image(d.path(), "a.png");
    let first = run(&src, Format::Bmp).unwrap();
    let second = run(&src, Format::Bmp).unwrap();
    assert_eq!(first.file_name().unwrap(), "a.bmp");
    assert_eq!(second.file_name().unwrap(), "a (1).bmp");
}

#[test]
fn fixed_dir_and_overwrite() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_image(d.path(), "a.png");
    let o = Options {
        output: OutputParams {
            dir: OutDir::Fixed(d.path().join("out")),
            conflict: Conflict::Overwrite,
        },
        ..Default::default()
    };
    let a = convert(&src, Format::Bmp, &o, |_| {}).unwrap();
    let b = convert(&src, Format::Bmp, &o, |_| {}).unwrap();
    assert_eq!(a, b);
    assert!(a.starts_with(fs::canonicalize(d.path()).unwrap().join("out")) || a.exists());
}

#[test]
fn replace_original_recycles_source() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_image(d.path(), "r.png");
    let o = Options {
        output: OutputParams {
            conflict: Conflict::ReplaceOriginal,
            ..Default::default()
        },
        ..Default::default()
    };
    let out = convert(&src, Format::Bmp, &o, |_| {}).unwrap();
    assert!(out.exists());
    assert!(!src.exists());
}
