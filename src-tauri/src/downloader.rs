use crate::metadata::{SearchResult, TrackMeta};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};

// -------------------- Search Cache (Binary Format) --------------------

fn get_cache_path(app: &AppHandle) -> PathBuf {
    let mut path = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    path.push("search_cache.bin");
    path
}

fn load_cache(app: &AppHandle) -> std::collections::HashMap<String, Vec<SearchResult>> {
    let path = get_cache_path(app);
    if let Ok(data) = std::fs::read(path) {
        bincode::deserialize(&data).unwrap_or_default()
    } else {
        std::collections::HashMap::new()
    }
}

fn save_cache(app: &AppHandle, cache: &std::collections::HashMap<String, Vec<SearchResult>>) {
    let path = get_cache_path(app);
    if let Ok(data) = bincode::serialize(cache) {
        let _ = std::fs::write(path, data);
    }
}

// -------------------- Search Logic --------------------

pub async fn search_tracks(app: &AppHandle, query: &str) -> Result<Vec<SearchResult>, String> {
    let cache_key = query.to_lowercase().trim().to_string();
    let mut cache = load_cache(app);

    if let Some(results) = cache.get(&cache_key) {
        return Ok(results.clone());
    }

    let results = search_youtube_fast(query).await?;

    if !results.is_empty() {
        cache.insert(cache_key, results.clone());
        if cache.len() > 200 {
            cache.clear();
        }
        save_cache(app, &cache);
    }

    Ok(results)
}

async fn search_youtube_fast(query: &str) -> Result<Vec<SearchResult>, String> {
    use rusty_ytdl::search::{SearchOptions, SearchResult as YtResult, YouTube};

    let youtube = YouTube::new().map_err(|e| format!("YT init failed: {e}"))?;
    let options = SearchOptions {
        limit: 10,
        ..Default::default()
    };
    let search_results = youtube
        .search(query, Some(&options))
        .await
        .map_err(|e| format!("YT search failed: {e}"))?;

    let mut results = Vec::new();
    for res in search_results {
        if let YtResult::Video(v) = res {
            // FIXED: Properly handle Option<String> for the year extraction
            let year = v.uploaded_at.as_ref().and_then(|uploaded_at| {
                if uploaded_at.len() >= 4 && uploaded_at[0..4].chars().all(|c| c.is_ascii_digit()) {
                    Some(uploaded_at[0..4].to_string())
                } else {
                    None
                }
            });

            let cover_url = v
                .thumbnails
                .first()
                .map(|t| t.url.clone())
                .or_else(|| Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", v.id)));

            results.push(SearchResult {
                id: v.id.clone(),
                title: v.title.clone(),
                artist: v.channel.name.clone(),
                cover_url,
                year,
                duration_ms: (v.duration as u64) * 1000,
            });
        }
    }
    Ok(results)
}

pub async fn resolve_spotify_to_youtube(spotify_url: &str) -> Result<String, String> {
    let encoded = urlencoding::encode(spotify_url);
    let api_url = format!("https://api.song.link/v1-alpha.1/links?url={}", encoded);

    let resp = reqwest::get(&api_url).await.map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(links) = json.get("linksByPlatform") {
        if let Some(yt) = links.get("youtube") {
            if let Some(u) = yt.get("url").and_then(|u| u.as_str()) {
                return Ok(u.to_string());
            }
        }
    }
    Err("No YouTube match found on Odesli".into())
}

pub async fn resolve_input(app: &AppHandle, input: &str) -> Result<Vec<SearchResult>, String> {
    let trimmed = input.trim();
    if trimmed.contains("open.spotify.com") {
        let yt_url = resolve_spotify_to_youtube(trimmed).await?;
        Ok(vec![SearchResult {
            id: yt_url.clone(),
            title: "Resolved from Spotify".into(),
            artist: "YouTube".into(),
            cover_url: None,
            year: None,
            duration_ms: 0,
        }])
    } else if trimmed.contains("youtube.com") || trimmed.contains("youtu.be") {
        Ok(vec![SearchResult {
            id: trimmed.to_string(),
            title: "YouTube Link".into(),
            artist: "User Provided".into(),
            cover_url: None,
            year: None,
            duration_ms: 0,
        }])
    } else {
        search_tracks(app, trimmed).await
    }
}

// -------------------- Download Logic --------------------

pub async fn download_track(
    app: &AppHandle,
    meta: &TrackMeta,
    final_path: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    #[cfg(not(target_os = "android"))]
    {
        return desktop_download(app, meta, final_path, cancel).await;
    }
    #[cfg(target_os = "android")]
    {
        return android_download(meta, final_path, cancel).await;
    }
}

#[cfg(not(target_os = "android"))]
async fn desktop_download(
    app: &AppHandle,
    meta: &TrackMeta,
    final_path: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    use crate::dependencies;
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }

    let yt = dependencies::yt_dlp_path(app)?;
    let ff = dependencies::ffmpeg_path(app)?;

    let url = if meta.id.starts_with("http") {
        meta.id.clone()
    } else {
        format!("https://www.youtube.com/watch?v={}", meta.id)
    };

    let out_base = final_path.with_extension("");
    let mut cmd = tokio::process::Command::new(&yt);

    cmd.arg("-x")
        .arg("--audio-format")
        .arg("mp3")
        .arg("--audio-quality")
        .arg("192K")
        .arg("--ffmpeg-location")
        .arg(&ff)
        .arg("--no-playlist")
        .arg("--embed-thumbnail")
        .arg("--convert-thumbnails")
        .arg("jpg")
        .arg("--embed-metadata")
        .arg("--extractor-args")
        .arg("youtube:player_client=default,-web_safari")
        .arg("-o")
        .arg(&out_base)
        .arg(&url);

    let status = cmd.status().await.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("yt-dlp download failed".into());
    }

    let _ = std::fs::remove_file(final_path.with_extension("webp"));
    let _ = std::fs::remove_file(final_path.with_extension("png"));
    let _ = std::fs::remove_file(final_path.with_extension("jpg"));

    tag_file(final_path, meta).await?;
    let size = std::fs::metadata(final_path).map(|m| m.len()).unwrap_or(0);
    Ok(size)
}

#[cfg(target_os = "android")]
async fn android_download(
    meta: &TrackMeta,
    final_path: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    use rusty_ytdl::{Video, VideoOptions, VideoQuality, VideoSearchOptions};

    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }

    let url = if meta.id.starts_with("http") {
        meta.id.clone()
    } else {
        format!("https://www.youtube.com/watch?v={}", meta.id)
    };

    let video_options = VideoOptions {
        quality: VideoQuality::Lowest,
        filter: VideoSearchOptions::Audio,
        ..Default::default()
    };

    let video =
        Video::new_with_options(&url, video_options).map_err(|e| format!("YT init failed: {e}"))?;
    let temp_path = final_path.with_extension("ytdl.tmp");

    video
        .download(&temp_path)
        .await
        .map_err(|e| format!("YT download failed: {e}"))?;

    if cancel.load(Ordering::Relaxed) {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err("Cancelled".into());
    }

    let pcm = decode_audio_file_to_pcm(&temp_path)?;
    let mp3_bytes = encode_pcm_to_mp3(&pcm)?;
    tokio::fs::write(final_path, &mp3_bytes)
        .await
        .map_err(|e| format!("Write MP3 error: {e}"))?;

    let _ = tokio::fs::remove_file(&temp_path).await;

    tag_file_android(final_path, meta).await?;
    Ok(mp3_bytes.len() as u64)
}

#[cfg(target_os = "android")]
fn decode_audio_file_to_pcm(path: &Path) -> Result<Vec<f32>, String> {
    use symphonia::core::audio::{AudioBufferRef, Signal};
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

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
            AudioBufferRef::S16(buf) => {
                let num_channels = buf.spec().channels.count();
                let num_frames = buf.frames();
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        pcm_data.push(buf.chan(ch)[frame] as f32 / 32768.0);
                    }
                }
            }
            _ => return Err("Unsupported audio format".into()),
        }
    }
    Ok(pcm_data)
}

#[cfg(target_os = "android")]
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

#[cfg(target_os = "android")]
async fn tag_file_android(path: &Path, meta: &TrackMeta) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::AudioFile;
    use lofty::file::TaggedFileExt;
    use lofty::picture::Picture;
    use lofty::tag::Accessor;
    use lofty::tag::Tag;

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

    let write_options = WriteOptions::new().use_id3v23(true);
    tagged_file
        .save_to_path(path, write_options)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(target_os = "android"))]
async fn tag_file(path: &Path, meta: &TrackMeta) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::AudioFile;
    use lofty::file::TaggedFileExt;
    use lofty::picture::Picture;
    use lofty::tag::Accessor;
    use lofty::tag::Tag;

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

    if tag.pictures().is_empty() {
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
    }

    let write_options = WriteOptions::new().use_id3v23(true);
    tagged_file
        .save_to_path(path, write_options)
        .map_err(|e| e.to_string())?;
    Ok(())
}
