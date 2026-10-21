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
    match target {
        Format::Wav => {
            let mut wav: Option<WavWriter<&mut W>> = None;
            let mut w = Some(w);
            decode(src, ext, progress, |rate, ch, samples| {
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
            decode(src, ext, progress, |rate, ch, samples| {
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
