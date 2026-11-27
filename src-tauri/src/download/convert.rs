//! Bandwidth saver: reuse an already-downloaded video as the audio source.
use std::path::Path;

pub async fn video_to_audio(
    video: &Path,
    audio: &Path,
    ffmpeg: &Path,
    cover_url: Option<&str>,
    title: &str,
    artist: &str,
) -> Result<(), String> {
    let cover: Option<std::path::PathBuf> = match cover_url {
        Some(url) => {
            let tmp = std::env::temp_dir().join(format!("d3khan_cover_{}.jpg", std::process::id()));
            match reqwest::Client::new().get(url).send().await {
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => {
                        if std::fs::write(&tmp, &bytes).is_ok() { Some(tmp) } else { None }
                    }
                    Err(_) => None,
                },
                Err(_) => None,
            }
        }
        None => None,
    };

    let mut args: Vec<String> = vec!["-i".into(), video.to_string_lossy().into()];
    if let Some(c) = &cover {
        args.push("-i".into());
        args.push(c.to_string_lossy().into());
        args.push("-map".into()); args.push("0:a:0".into());
        args.push("-map".into()); args.push("1:v:0".into());
        args.push("-c:v".into()); args.push("mjpeg".into());
        args.push("-disposition:v".into()); args.push("attached_pic".into());
        args.push("-id3v2_version".into()); args.push("3".into());
    } else {
        args.push("-vn".into());
    }
    args.push("-acodec".into()); args.push("libmp3lame".into());
    args.push("-q:a".into()); args.push("2".into());
    args.push("-metadata".into()); args.push(format!("title={title}"));
    args.push("-metadata".into()); args.push(format!("artist={artist}"));
    args.push("-y".into());
    args.push(audio.to_string_lossy().into());

    let out = tokio::process::Command::new(ffmpeg)
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;

    let ok = if out.status.success() {
        true
    } else if cover.is_some() {
        let fb: Vec<String> = vec![
            "-i".into(), video.to_string_lossy().into(),
            "-vn".into(), "-acodec".into(), "libmp3lame".into(),
            "-q:a".into(), "2".into(),
            "-metadata".into(), format!("title={title}"),
            "-metadata".into(), format!("artist={artist}"),
            "-y".into(), audio.to_string_lossy().into(),
        ];
        match tokio::process::Command::new(ffmpeg).args(&fb).output().await {
            Ok(r) => r.status.success(),
            Err(_) => false,
        }
    } else {
        false
    };

    if let Some(c) = &cover {
        let _ = std::fs::remove_file(c);
    }
    if !ok {
        return Err(format!("ffmpeg conversion failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}
