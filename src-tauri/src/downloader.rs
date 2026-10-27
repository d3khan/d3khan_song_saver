use crate::metadata::TrackMeta;
use lofty::file::AudioFile;
use lofty::picture::Picture;
use lofty::tag::Tag;
use soundcloud_rs::SoundcloudIdentifier;
use soundcloud_rs::query::TracksQuery;
use soundcloud_rs::response::StreamType;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub async fn soundcloud_client() -> Result<soundcloud_rs::Client, String> {
    soundcloud_rs::Client::new()
        .await
        .map_err(|e| format!("SoundCloud client init failed: {e}"))
}

pub async fn search_soundcloud(query: &str) -> Result<Vec<(String, String, String)>, String> {
    let query = query.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        rt.block_on(async move {
            let client = soundcloud_client().await?;
            let tracks_query = TracksQuery {
                q: Some(query),
                limit: Some(20),
                ..Default::default()
            };
            let results = client
                .search_tracks(Some(&tracks_query))
                .await
                .map_err(|e| format!("SoundCloud search failed: {e}"))?;

            let mapped: Vec<(String, String, String)> = results
                .collection
                .into_iter()
                .map(|r| {
                    let id = r.id.map(|i| i.to_string()).unwrap_or_default();
                    let title = r.title.unwrap_or_default();
                    let artist = r
                        .user
                        .as_ref()
                        .and_then(|u| u.username.clone())
                        .unwrap_or_default();
                    (id, title, artist)
                })
                .collect();

            Ok(mapped)
        })
    })
    .await
    .map_err(|e| format!("Search task panicked: {e}"))?
}

pub async fn download_track(
    meta: &TrackMeta,
    final_path: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    let meta = meta.clone();
    let final_path = final_path.to_path_buf();

    tauri::async_runtime::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        rt.block_on(async move {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }

            let client = soundcloud_client().await?;
            let track_id: i64 = meta.id.parse().map_err(|_| "Invalid track ID")?;

            let track = client
                .get_track(&SoundcloudIdentifier::Id(track_id))
                .await
                .map_err(|e| format!("Fetch track failed: {e}"))?;

            let transcodings = track
                .media
                .as_ref()
                .and_then(|m| m.transcodings.as_ref())
                .ok_or("No media transcodings found")?;

            let prog_url = transcodings.iter().find_map(|t| {
                let is_prog = t.format.as_ref().and_then(|f| f.protocol.as_ref())
                    == Some(&StreamType::Progressive);
                if is_prog { t.url.clone() } else { None }
            });

            if let Some(url) = prog_url {
                let bytes = download_raw_bytes(&url, cancel.clone()).await?;
                tokio::fs::write(&final_path, &bytes)
                    .await
                    .map_err(|e| format!("Write error: {e}"))?;
                tag_file(&final_path, &meta).await?;
                return Ok(bytes.len() as u64);
            }

            let hls_url = transcodings.iter().find_map(|t| {
                let is_hls =
                    t.format.as_ref().and_then(|f| f.protocol.as_ref()) == Some(&StreamType::Hls);
                if is_hls { t.url.clone() } else { None }
            });

            if let Some(url) = hls_url {
                let aac_bytes = download_raw_bytes(&url, cancel.clone()).await?;
                let pcm = decode_aac_to_pcm(&aac_bytes)?;
                let mp3_bytes = encode_pcm_to_mp3(&pcm)?;
                tokio::fs::write(&final_path, &mp3_bytes)
                    .await
                    .map_err(|e| format!("Write error: {e}"))?;
                tag_file(&final_path, &meta).await?;
                return Ok(mp3_bytes.len() as u64);
            }

            Err("No available stream (neither Progressive MP3 nor HLS AAC)".into())
        })
    })
    .await
    .map_err(|e| format!("Download task panicked: {e}"))?
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
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Download bytes error: {e}"))?;
    Ok(bytes.to_vec())
}

fn decode_aac_to_pcm(bytes: &[u8]) -> Result<Vec<f32>, String> {
    use symphonia::core::audio::AudioBufferRef;
    use symphonia::core::audio::Signal;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let mss = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    let mut hint = Hint::new();
    hint.with_extension("aac");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("No audio track")?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;

    let mut pcm_data: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet).map_err(|e| e.to_string())? {
            AudioBufferRef::F32(buf) => {
                let num_channels = buf.spec().channels.count();
                let num_frames = buf.frames();
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        pcm_data.push(buf.chan(ch)[frame]);
                    }
                }
            }
            _ => return Err("Unsupported audio format from Symphonia".into()),
        }
    }
    Ok(pcm_data)
}

fn encode_pcm_to_mp3(pcm: &[f32]) -> Result<Vec<u8>, String> {
    use shine_rs::mp3_encoder::{Mp3Encoder, Mp3EncoderConfig, StereoMode};

    let config = Mp3EncoderConfig::new()
        .sample_rate(44100)
        .bitrate(128)
        .channels(2)
        .stereo_mode(StereoMode::Stereo);

    let mut encoder = Mp3Encoder::new(config).map_err(|e| format!("Shine init error: {:?}", e))?;

    let pcm_i16: Vec<i16> = pcm.iter().map(|&v| (v * 32767.0) as i16).collect();

    let chunks = encoder
        .encode_interleaved(&pcm_i16)
        .map_err(|e| format!("Shine encode error: {:?}", e))?;
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(&chunk);
    }

    let final_data = encoder
        .finish()
        .map_err(|e| format!("Shine finish error: {:?}", e))?;
    out.extend_from_slice(&final_data);

    Ok(out)
}

async fn tag_file(path: &Path, meta: &TrackMeta) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::TaggedFileExt;
    use lofty::tag::Accessor;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut tagged_file = lofty::read_from(&mut file).map_err(|e| e.to_string())?;

    let tag = match tagged_file.primary_tag_mut() {
        Some(t) => t,
        None => {
            let tag_type = tagged_file.primary_tag_type();
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.primary_tag_mut().unwrap()
        }
    };

    tag.set_title(meta.title.clone());
    tag.set_artist(meta.artists.join(", "));
    if !meta.album.is_empty() {
        tag.set_album(meta.album.clone());
    }

    if let Some(cover_url) = &meta.cover_url {
        if let Ok(resp) = reqwest::get(cover_url).await {
            if let Ok(img) = resp.bytes().await {
                let mut cursor = std::io::Cursor::new(img.to_vec());
                if let Ok(pic) = Picture::from_reader(&mut cursor) {
                    tag.push_picture(pic);
                }
            }
        }
    }

    tagged_file
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| e.to_string())?;
    Ok(())
}
