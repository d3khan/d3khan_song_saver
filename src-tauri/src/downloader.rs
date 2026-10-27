//! Download pipeline: SoundCloud (soundcloud-rs) -> symphonia (AAC decode) -> shine-rs (MP3 encode) -> lofty tags.
// No ffmpeg/yt-dlp; pure Rust pipeline.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, TagType};
use lofty::picture::{MimeType, Picture, PictureType};

use crate::metadata::TrackMeta;

pub async fn soundcloud_client() -> Result<soundcloud_rs::Client, String> {
    soundcloud_rs::Client::new()
        .await
        .map_err(|e| format!("SoundCloud client init failed: {e}"))
}

pub async fn search_soundcloud(query: &str) -> Result<Vec<(String, String, String)>, String> {
    let client = soundcloud_client().await?;
    let results = client
        .search_tracks(query)
        .await
        .map_err(|e| format!("SoundCloud search failed: {e}"))?;
    let mapped: Vec<(String, String, String)> = results.collection
        .into_iter()
        .map(|r| (r.id.clone(), r.title.clone(), r.user.username.clone()))
        .collect();
    Ok(mapped)
}

/// Section A — SoundCloud download with Progressive/HLS fallback.
pub async fn download_track(
    meta: &TrackMeta,
    final_path: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }

    let client = soundcloud_client().await?;
    let track = client.resolve_track(&meta.id).await.map_err(|e| e.to_string())?;

    // Progressive direct MP3 attempt
    if let Some(stream) = track.stream() {
        if let Some(prog_url) = stream.progressive.as_ref().map(|p| p.url.clone()) {
            let bytes = download_raw_bytes(&prog_url, cancel.clone()).await?;
            tokio::fs::write(final_path, bytes.clone()).await.map_err(|e| format!("Write error: {e}"))?;
            tag_file(final_path, meta).await?;
            return Ok(bytes.len() as u64);
        }
    }

    // HLS AAC fallback
    if let Some(stream) = track.stream() {
        if let Some(hls_url) = stream.hls.as_ref().map(|h| h.url.clone()) {
            let aac_bytes = download_raw_bytes(&hls_url, cancel.clone()).await?;
            let pcm = decode_aac_to_pcm(&aac_bytes)?;
            let mp3_bytes = encode_pcm_to_mp3(&pcm)?;
            tokio::fs::write(final_path, mp3_bytes.clone()).await.map_err(|e| format!("Write error: {e}"))?;
            tag_file(final_path, meta).await?;
            return Ok(mp3_bytes.len() as u64);
        }
    }

    Err("No available stream (neither Progressive MP3 nor HLS AAC)".into())
}

async fn download_raw_bytes(url: &str, cancel: Arc<AtomicBool>) -> Result<Vec<u8>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    let resp = reqwest::get(url)
        .await
        .map_err(|e| format!("Download request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download HTTP error: {e}"))?;
    let bytes = resp.bytes().await.map_err(|e| format!("Download bytes error: {e}"))?;
    Ok(bytes.to_vec())
}

/// Section B — Symphonia decode AAC/MP4 bytes to PCM Vec<f32>.
fn decode_aac_to_pcm(bytes: &[u8]) -> Result<Vec<f32>, String> {
    use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("mp4");

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Symphonia probe error: {e}"))?;

    let mut format = probed.format;
    let track = format.tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio track found".to_string())?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Symphonia decoder creation error: {e}"))?;

    let mut pcm_f32: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Symphonia packet error: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(AudioBufferRef::F32(buf)) => {
                let num_channels = buf.spec().channels.count();
                let num_frames = buf.frames();
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        pcm_f32.push(buf.planes().planes().get(ch).map(|p| p[frame]).unwrap_or(0.0));
                    }
                }
            }
            Ok(_) => return Err("Unsupported sample format from symphonia".into()),
            Err(e) => return Err(format!("Symphonia decode error: {e}")),
        }
    }
    Ok(pcm_f32)
}

/// Section C — Shine-RS pure-Rust MP3 encoding (44100 Hz, Stereo, 128 kbps).
fn encode_pcm_to_mp3(pcm: &[f32]) -> Result<Vec<u8>, String> {
    use shine_rs::mp3_encoder::{Mp3Encoder, Mp3EncoderConfig, StereoMode};

    let config = Mp3EncoderConfig::new()
        .sample_rate(44100)
        .bitrate(128)
        .channels(2)
        .stereo_mode(StereoMode::Stereo);

    let mut encoder = Mp3Encoder::new(config)
        .map_err(|e| format!("Shine-RS encoder init error: {e}"))?;

    // Interleaved stereo PCM; shine-rs expects interleaved samples.
    let mp3_data = encoder.encode_interleaved(&(pcm.iter().map(|&v| (v * 32767.0) as i16).collect::<Vec<i16>>()[..]))
        .map_err(|e| format!("Shine-RS encode error: {e}"))?;

    let final_data = encoder.finish()
        .map_err(|e| format!("Shine-RS finalize error: {e}"))?;

    Ok(final_data)
}

/// ID3v2.3 tagging with lofty (cover art best-effort, never fails the download).
async fn tag_file(path: &Path, meta: &TrackMeta) -> Result<(), String> {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title(meta.title.clone());
    tag.set_artist(meta.artists.join(", "));
    tag.set_album(meta.album.clone());
    tag.set_track(meta.track_number);

    if !meta.artists.is_empty() {
        tag.insert_text(ItemKey::AlbumArtist, meta.artists.join(", "));
    }

    // Best-effort cover art download
    if let Some(url) = meta.cover_url.as_deref() {
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            if let Ok(resp) = client.get(url).send().await {
                if resp.status().is_success() {
                    if let Ok(img) = resp.bytes().await {
                        let data = img.to_vec();
                        if !data.is_empty() {
                            let mime = if data.starts_with(&[0xFF, 0xD8]) {
                                lofty::picture::MimeType::Jpeg
                            } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
                                lofty::picture::MimeType::Png
                            } else {
                                lofty::picture::MimeType::Png("image/png".to_string())
                            };
                            if let Ok(pic) = lofty::picture::Picture::from_bytes(data) { tag.push_picture(pic); }
                            tag.push_picture(picture);
                        }
                    }
                }
            }
        }
    }

    tag.save_to_path(path, lofty::config::WriteOptions::new().use_id3v23(true))
        .map_err(|e| format!("Tag write error: {e}"))?;
    Ok(())
}
