use std::fs::File;
use std::io::{ErrorKind, Seek, Write};
use std::path::Path;

use flacenc::component::BitRepr;
use flacenc::error::Verify;
use hound::{SampleFormat, WavSpec, WavWriter};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::{ConvertError, Format};

/// FLAC encoding holds all samples in memory: ~100M samples = 400 MB (about 18 min of 44.1 kHz stereo).
const MAX_FLAC_SAMPLES: usize = 100_000_000;
/// Tolerated undecodable packets before the file counts as corrupt.
const MAX_BAD_PACKETS: u32 = 100;

pub fn convert<W: Write + Seek>(
    src: &Path,
    ext: &str,
    target: Format,
    w: &mut W,
    progress: &mut impl FnMut(f32),
) -> Result<(), ConvertError> {
    encode(target, w, |sink| decode(src, ext, progress, sink))
}

type Sink<'a> = &'a mut dyn FnMut(u32, u16, &[i16]) -> Result<(), ConvertError>;

/// Several files back to back as one WAV or FLAC. Files that differ from the first in
/// sample rate or channel count are adapted to it.
pub fn merge<W: Write + Seek>(
    srcs: &[(&Path, &str)],
    target: Format,
    w: &mut W,
    progress: &mut impl FnMut(f32),
) -> Result<(), ConvertError> {
    let n = srcs.len() as f32;
    let mut spec: Option<(u32, u16)> = None;
    encode(target, w, |sink| {
        for (i, (src, ext)) in srcs.iter().enumerate() {
            let mut p = |f: f32| progress((i as f32 + f) / n);
            decode(src, ext, &mut p, |rate, ch, samples| {
                let (r, c) = *spec.get_or_insert((rate, ch));
                if (rate, ch) == (r, c) {
                    sink(r, c, samples)
                } else {
                    sink(r, c, &adapt(samples, rate, ch, r, c))
                }
            })?;
            progress((i + 1) as f32 / n);
        }
        Ok(())
    })
}

// ponytail: nearest-sample resampling, per decoded packet; audible only as slight aliasing
// when sample rates differ. Swap in linear interpolation if that ever matters.
fn adapt(samples: &[i16], rate: u32, ch: u16, to_rate: u32, to_ch: u16) -> Vec<i16> {
    let (ch, to_ch) = (usize::from(ch.max(1)), usize::from(to_ch.max(1)));
    let frames = samples.len() / ch;
    if frames == 0 {
        return vec![];
    }
    let out_frames = (frames as u64 * u64::from(to_rate) / u64::from(rate.max(1))) as usize;
    let mut out = Vec::with_capacity(out_frames * to_ch);
    for o in 0..out_frames {
        let f = ((o as u64 * u64::from(rate)) / u64::from(to_rate.max(1))) as usize;
        let frame = &samples[f.min(frames - 1) * ch..][..ch];
        for c in 0..to_ch {
            out.push(if ch == to_ch {
                frame[c]
            } else if to_ch == 1 {
                (frame.iter().map(|&s| i32::from(s)).sum::<i32>() / ch as i32) as i16
            } else {
                frame[c % ch]
            });
        }
    }
    out
}

fn encode<W: Write + Seek>(
    target: Format,
    w: &mut W,
    drive: impl FnOnce(Sink) -> Result<(), ConvertError>,
) -> Result<(), ConvertError> {
    match target {
        Format::Wav => {
            let mut wav: Option<WavWriter<&mut W>> = None;
            let mut w = Some(w);
            drive(&mut |rate, ch, samples| {
                if wav.is_none() {
                    let spec = WavSpec {
                        channels: ch,
                        sample_rate: rate,
                        bits_per_sample: 16,
                        sample_format: SampleFormat::Int,
                    };
                    let out = w.take().expect("created once");
                    wav = Some(WavWriter::new(out, spec).map_err(wav_err)?);
                }
                let wr = wav.as_mut().expect("just created");
                for &s in samples {
                    wr.write_sample(s).map_err(wav_err)?;
                }
                Ok(())
            })?;
            wav.ok_or(ConvertError::Corrupt)?
                .finalize()
                .map_err(wav_err)
        }
        Format::Flac => {
            let mut all: Vec<i32> = Vec::new();
            let mut fmt = (0u32, 0u16);
            drive(&mut |rate, ch, samples| {
                if all.len() + samples.len() > MAX_FLAC_SAMPLES {
                    return Err(ConvertError::TooLarge);
                }
                fmt = (rate, ch);
                all.extend(samples.iter().map(|&s| i32::from(s)));
                Ok(())
            })?;
            let (rate, ch) = fmt;
            let config = flacenc::config::Encoder::default()
                .into_verified()
                .map_err(|e| ConvertError::Encode(format!("{:?}", e.1)))?;
            let source =
                flacenc::source::MemSource::from_samples(&all, usize::from(ch), 16, rate as usize);
            let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
                .map_err(|e| ConvertError::Encode(e.to_string()))?;
            let mut sink = flacenc::bitsink::ByteSink::new();
            stream
                .write(&mut sink)
                .map_err(|e| ConvertError::Encode(e.to_string()))?;
            // flacenc records the short last frame as min block size, which makes fixed-size
            // streams look variable-blocked and symphonia rejects them; min = max is valid.
            let mut bytes = sink.as_slice().to_vec();
            bytes.copy_within(10..12, 8);
            w.write_all(&bytes)?;
            Ok(())
        }
        _ => Err(ConvertError::Unsupported),
    }
}

fn wav_err(e: hound::Error) -> ConvertError {
    match e {
        hound::Error::IoError(e) => e.into(),
        e => ConvertError::Encode(e.to_string()),
    }
}

fn sym_err(e: SymError) -> ConvertError {
    match e {
        SymError::IoError(e) => e.into(),
        SymError::Unsupported(_) => ConvertError::Unsupported,
        SymError::LimitError(_) => ConvertError::TooLarge,
        _ => ConvertError::Corrupt,
    }
}

/// Streams decoded interleaved i16 samples to `sink(rate, channels, samples)`.
fn decode(
    src: &Path,
    ext: &str,
    progress: &mut impl FnMut(f32),
    mut sink: impl FnMut(u32, u16, &[i16]) -> Result<(), ConvertError>,
) -> Result<(), ConvertError> {
    let mss = MediaSourceStream::new(Box::new(File::open(src)?), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext);
    // an unrecognizable stream under an audio extension is a damaged file
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| match e {
            SymError::IoError(e) => e.into(),
            _ => ConvertError::Corrupt,
        })?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(ConvertError::Unsupported)?;
    let (id, total) = (track.id, track.codec_params.n_frames);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(sym_err)?;

    let (mut bad, mut frames) = (0u32, 0u64);
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(sym_err(e)),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => {
                bad += 1;
                if bad > MAX_BAD_PACKETS {
                    return Err(ConvertError::Corrupt);
                }
                continue;
            }
            Err(e) => return Err(sym_err(e)),
        };
        let spec = *decoded.spec();
        let mut buf = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        let ch = spec.channels.count() as u16;
        sink(spec.rate, ch, buf.samples())?;
        frames += (buf.samples().len() / usize::from(ch.max(1))) as u64;
        if let Some(t) = total.filter(|&t| t > 0) {
            progress((frames as f32 / t as f32).min(0.99));
        }
    }
    if frames == 0 {
        return Err(ConvertError::Corrupt);
    }
    Ok(())
}
