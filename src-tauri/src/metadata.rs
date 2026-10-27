//! Spotify link parsing and metadata fetching via `rspotify`.
//!
//! `parse_spotify_url` is a pure helper (no network); `fetch_collection` asks the
//! Spotify Web API for a track, album, or playlist and flattens it into the
//! shared [`Collection`] / [`TrackMeta`] types used by the job runner.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use rspotify::AuthCodePkceSpotify;
use rspotify::clients::BaseClient;
use rspotify::model::{
    AlbumId, FullAlbum, FullTrack, Id, Image, PlayableItem, PlaylistId, SimplifiedArtist,
    SimplifiedTrack, TrackId,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind {
    Track,
    Album,
    Playlist,
}

/// Everything Spotify exposes about a track that is worth writing into the MP3.
#[derive(Clone, Debug, Default)]
pub struct TrackMeta {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_id: Option<String>,
    pub album_artists: Vec<String>,
    pub track_number: u32,
    pub total_tracks: Option<u32>,
    pub disc_number: u32,
    pub duration_ms: u64,
    /// Release date as Spotify reports it (`YYYY`, `YYYY-MM`, or `YYYY-MM-DD`).
    pub release_date: Option<String>,
    pub isrc: Option<String>,
    pub label: Option<String>,
    pub copyright: Option<String>,
    pub genres: Vec<String>,
    pub explicit: bool,
    pub spotify_url: Option<String>,
    pub cover_url: Option<String>,
    /// 1-based position among accepted playlist tracks; `0` otherwise.
    pub position: usize,
}

#[derive(Clone, Debug)]
pub struct Collection {
    pub kind: CollectionKind,
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
    pub tracks: Vec<TrackMeta>,
    pub source_url: String,
}

/// Page size used when walking playlist items.
const PLAYLIST_PAGE: u32 = 100;
/// Page size used when walking album tracks.
const ALBUM_PAGE: u32 = 50;

static URL_RE: OnceLock<Regex> = OnceLock::new();
static URI_RE: OnceLock<Regex> = OnceLock::new();

fn url_regex() -> &'static Regex {
    URL_RE.get_or_init(|| {
        // Matches `.../track/<id>`, `/intl-de/album/<id>`, query strings, etc.
        Regex::new(r"(track|album|playlist)/([A-Za-z0-9]{22})").expect("valid Spotify URL regex")
    })
}

fn uri_regex() -> &'static Regex {
    URI_RE.get_or_init(|| {
        Regex::new(r"spotify:(track|album|playlist):([A-Za-z0-9]{22})")
            .expect("valid Spotify URI regex")
    })
}

/// Accepts open.spotify.com URLs and spotify: URIs; rejects artist/episode/show.
pub fn parse_spotify_url(input: &str) -> Result<(CollectionKind, String), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Paste a Spotify track, album, or playlist link.".to_string());
    }

    if let Some(captures) = url_regex().captures(trimmed) {
        if let Some(id_match) = captures.get(2) {
            if !followed_by_alnum(trimmed, id_match.end()) {
                let kind = kind_from_str(&captures[1]);
                return Ok((kind, id_match.as_str().to_string()));
            }
        }
    }

    if let Some(captures) = uri_regex().captures(trimmed) {
        if let Some(id_match) = captures.get(2) {
            if !followed_by_alnum(trimmed, id_match.end()) {
                let kind = kind_from_str(&captures[1]);
                return Ok((kind, id_match.as_str().to_string()));
            }
        }
    }

    if let Some(label) = unsupported_kind(trimmed) {
        return Err(format!(
            "{label} links aren't supported. Use a track, album, or playlist link."
        ));
    }

    Err("That doesn't look like a Spotify track, album, or playlist link.".to_string())
}

fn kind_from_str(kind: &str) -> CollectionKind {
    match kind {
        "track" => CollectionKind::Track,
        "album" => CollectionKind::Album,
        _ => CollectionKind::Playlist,
    }
}

/// Rejects IDs longer than 22 chars instead of silently truncating them.
fn followed_by_alnum(input: &str, byte_end: usize) -> bool {
    input[byte_end..]
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
}

fn unsupported_kind(input: &str) -> Option<&'static str> {
    const UNSUPPORTED: [(&str, &str); 3] = [
        ("artist", "Artist"),
        ("episode", "Episode"),
        ("show", "Show"),
    ];

    for (needle, label) in UNSUPPORTED {
        if input.contains(&format!("{needle}/")) || input.contains(&format!("spotify:{needle}:")) {
            return Some(label);
        }
    }
    None
}

pub async fn fetch_collection(
    client: &AuthCodePkceSpotify,
    kind: CollectionKind,
    id: &str,
    source_url: &str,
) -> Result<Collection, String> {
    let mut collection = match kind {
        CollectionKind::Playlist => fetch_playlist(client, id, source_url).await?,
        CollectionKind::Album => fetch_album(client, id, source_url).await?,
        CollectionKind::Track => fetch_track(client, id, source_url).await?,
    };

    // Album pages already carry label/copyright/genre for their own tracks; playlist
    // items and single tracks only know a simplified album, so top those up.
    if matches!(kind, CollectionKind::Playlist | CollectionKind::Track) {
        enrich_tracks_with_albums(client, &mut collection.tracks).await;
    }

    Ok(collection)
}

/// Batch-fetches the albums behind `tracks` (20 IDs per request) and copies the
/// fields Spotify only exposes on the full album object. Best-effort: a failed
/// request leaves the tracks with plain metadata instead of failing the job.
async fn enrich_tracks_with_albums(client: &AuthCodePkceSpotify, tracks: &mut [TrackMeta]) {
    let mut album_ids: Vec<String> = Vec::new();
    for track in tracks.iter() {
        if let Some(album_id) = &track.album_id {
            if !album_ids.iter().any(|known| known == album_id) {
                album_ids.push(album_id.clone());
            }
        }
    }

    for chunk in album_ids.chunks(20) {
        let albums = fetch_albums(client, chunk).await;
        if albums.is_empty() {
            continue;
        }

        let by_id: HashMap<String, FullAlbum> = albums
            .into_iter()
            .map(|album| (album.id.id().to_string(), album))
            .collect();

        for track in tracks.iter_mut() {
            let Some(album) = track
                .album_id
                .as_ref()
                .and_then(|album_id| by_id.get(album_id))
            else {
                continue;
            };
            apply_album_metadata(track, album, false);
        }
    }
}

/// Batch album lookup with a per-album fallback: Spotify has been retiring the
/// multi-album endpoint for some app tiers, and losing it must not cost the tags.
#[allow(deprecated)]
async fn fetch_albums(client: &AuthCodePkceSpotify, ids: &[String]) -> Vec<FullAlbum> {
    let album_ids: Vec<AlbumId<'static>> = ids
        .iter()
        .filter_map(|id| AlbumId::from_id(id.clone()).ok())
        .collect();
    if album_ids.is_empty() {
        return Vec::new();
    }

    match client.albums(album_ids, None).await {
        Ok(albums) => return albums,
        Err(error) => eprintln!("Batch album lookup unavailable ({error}); trying one by one"),
    }

    let mut albums = Vec::new();
    for id in ids {
        let Ok(album_id) = AlbumId::from_id(id.clone()) else {
            continue;
        };
        match client.album(album_id, None).await {
            Ok(album) => albums.push(album),
            Err(error) => eprintln!("Album metadata skipped for {id}: {error}"),
        }
    }
    albums
}

/// Copies album-level fields onto a track. `from_album_tracks` marks a track that
/// belongs to the album being fetched, where cover/artists are authoritative.
#[allow(deprecated)]
fn apply_album_metadata(track: &mut TrackMeta, album: &FullAlbum, from_album_tracks: bool) {
    if track.album_artists.is_empty() {
        track.album_artists = artist_names(&album.artists);
    }
    if track.album.is_empty() {
        track.album = album.name.clone();
    }
    if track.release_date.is_none() {
        let date = album.release_date.trim();
        if !date.is_empty() {
            track.release_date = Some(date.to_string());
        }
    }
    if track.total_tracks.is_none() && album.tracks.total > 0 {
        track.total_tracks = Some(album.tracks.total);
    }
    if track.label.is_none() {
        track.label = album
            .label
            .as_ref()
            .map(|label| label.trim())
            .filter(|label| !label.is_empty())
            .map(str::to_string);
    }
    if track.copyright.is_none() {
        track.copyright = album
            .copyrights
            .first()
            .map(|copyright| copyright.text.trim())
            .filter(|text| !text.is_empty())
            .map(str::to_string);
    }
    if track.genres.is_empty() {
        track.genres = album
            .genres
            .iter()
            .map(|genre| genre.trim())
            .filter(|genre| !genre.is_empty())
            .take(3)
            .map(str::to_string)
            .collect();
    }
    if from_album_tracks && track.cover_url.is_none() {
        track.cover_url = image_url(&album.images);
    }
}

async fn fetch_playlist(
    client: &AuthCodePkceSpotify,
    id: &str,
    source_url: &str,
) -> Result<Collection, String> {
    let playlist_id =
        PlaylistId::from_id(id).map_err(|_| "That playlist link isn't valid.".to_string())?;

    // Fetch the first page before the summary: it validates access and lets rspotify
    // refresh an expired token. `client.playlist()` is not used because Spotify now
    // omits the `tracks` field for playlists the account cannot read, and rspotify's
    // `FullPlaylist` deserializer panics on that response.
    let mut page = client
        .playlist_items_manual(
            playlist_id.clone(),
            None,
            None,
            Some(PLAYLIST_PAGE),
            Some(0),
        )
        .await
        .map_err(|error| spotify_error("playlist", error))?;

    let (name, cover_url) = fetch_playlist_summary(client, id).await;

    let mut tracks: Vec<TrackMeta> = Vec::new();
    let mut offset: u32 = 0;

    loop {
        // `next` may be `Some` even on the final page, so an empty page is the
        // hard stop that also prevents an infinite loop.
        let page_len = page.items.len();
        if page_len == 0 {
            break;
        }
        let has_next = page.next.is_some();

        for item in page.items {
            // Local files and podcast episodes are not downloadable music tracks.
            if item.is_local {
                continue;
            }
            let Some(PlayableItem::Track(track)) = item.item else {
                continue;
            };
            let Some(mut meta) = track_meta_from_full(&track, tracks.len() + 1, None) else {
                continue;
            };
            if meta.cover_url.is_none() {
                meta.cover_url = cover_url.clone();
            }
            tracks.push(meta);
        }

        if !has_next {
            break;
        }
        offset = offset.saturating_add(page_len as u32);
        page = client
            .playlist_items_manual(
                playlist_id.clone(),
                None,
                None,
                Some(PLAYLIST_PAGE),
                Some(offset),
            )
            .await
            .map_err(|error| spotify_error("playlist", error))?;
    }

    Ok(Collection {
        kind: CollectionKind::Playlist,
        id: id.to_string(),
        name,
        cover_url,
        tracks,
        source_url: source_url.to_string(),
    })
}

/// Playlist name + cover without deserializing the whole playlist object. Returns a
/// readable fallback name when Spotify denies or omits the metadata.
async fn fetch_playlist_summary(
    client: &AuthCodePkceSpotify,
    id: &str,
) -> (String, Option<String>) {
    let fallback = format!("Playlist {id}");

    for attempt in 0..2 {
        let token = client
            .token
            .lock()
            .await
            .ok()
            .and_then(|guard| guard.clone());
        let Some(token) = token else {
            return (fallback, None);
        };

        let response = reqwest::Client::new()
            .get(format!("https://api.spotify.com/v1/playlists/{id}"))
            .query(&[("fields", "name,images")])
            .bearer_auth(&token.access_token)
            .send()
            .await;

        let Ok(response) = response else {
            return (fallback, None);
        };

        // This request bypasses rspotify, so an invalidated access token shows up
        // as a plain 401. Refresh once instead of silently falling back to
        // "Playlist <id>" (which would become the folder name).
        if response.status().as_u16() == 401 && attempt == 0 {
            if crate::auth::refresh_now(client).await {
                continue;
            }
            return (fallback, None);
        }
        if !response.status().is_success() {
            return (fallback, None);
        }
        let Ok(value) = response.json::<serde_json::Value>().await else {
            return (fallback, None);
        };

        let name = value
            .get("name")
            .and_then(|name| name.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback.as_str())
            .to_string();
        let cover_url = value
            .get("images")
            .and_then(|images| images.as_array())
            .and_then(|images| images.first())
            .and_then(|image| image.get("url"))
            .and_then(|url| url.as_str())
            .map(str::to_string);

        return (name, cover_url);
    }

    (fallback, None)
}

async fn fetch_album(
    client: &AuthCodePkceSpotify,
    id: &str,
    source_url: &str,
) -> Result<Collection, String> {
    let album_id = AlbumId::from_id(id).map_err(|_| "That album link isn't valid.".to_string())?;

    let album = client
        .album(album_id.clone(), None)
        .await
        .map_err(|error| spotify_error("album", error))?;

    let name = album.name.clone();
    let cover_url = image_url(&album.images);

    let mut tracks: Vec<TrackMeta> = Vec::new();
    let mut offset: u32 = 0;

    loop {
        let page = client
            .album_track_manual(album_id.clone(), None, Some(ALBUM_PAGE), Some(offset))
            .await
            .map_err(|error| spotify_error("album", error))?;

        let page_len = page.items.len();
        if page_len == 0 {
            break;
        }
        let has_next = page.next.is_some();

        for track in &page.items {
            if let Some(meta) = track_meta_from_simplified(track, &album, cover_url.as_deref()) {
                tracks.push(meta);
            }
        }

        if !has_next {
            break;
        }
        offset = offset.saturating_add(page_len as u32);
    }

    Ok(Collection {
        kind: CollectionKind::Album,
        id: id.to_string(),
        name,
        cover_url,
        tracks,
        source_url: source_url.to_string(),
    })
}

async fn fetch_track(
    client: &AuthCodePkceSpotify,
    id: &str,
    source_url: &str,
) -> Result<Collection, String> {
    let track_id = TrackId::from_id(id).map_err(|_| "That track link isn't valid.".to_string())?;

    let track = client
        .track(track_id, None)
        .await
        .map_err(|error| spotify_error("track", error))?;

    let meta = track_meta_from_full(&track, 0, Some(id))
        .ok_or_else(|| "Spotify returned a track without an ID.".to_string())?;

    Ok(Collection {
        kind: CollectionKind::Track,
        id: id.to_string(),
        // Jobs only use folder names for albums/playlists; a single track is
        // named after its title.
        name: track.name.clone(),
        cover_url: meta.cover_url.clone(),
        tracks: vec![meta],
        source_url: source_url.to_string(),
    })
}

/// Builds a [`TrackMeta`] from a full track (playlist items and single tracks).
///
/// Returns `None` when the track has no usable ID and no `fallback_id` is
/// provided, so callers can skip local/unavailable entries.
fn track_meta_from_full(
    track: &FullTrack,
    position: usize,
    fallback_id: Option<&str>,
) -> Option<TrackMeta> {
    let id = match track.id.as_ref() {
        Some(track_id) => track_id.id().to_string(),
        None => fallback_id?.to_string(),
    };

    Some(TrackMeta {
        id,
        title: track.name.clone(),
        artists: artist_names(&track.artists),
        album: track.album.name.clone(),
        album_id: track
            .album
            .id
            .as_ref()
            .map(|album_id| album_id.id().to_string()),
        album_artists: artist_names(&track.album.artists),
        track_number: track.track_number,
        total_tracks: None,
        disc_number: disc_to_u32(track.disc_number),
        duration_ms: duration_ms(&track.duration),
        release_date: clean_date(track.album.release_date.as_deref()),
        isrc: track.external_ids.get("isrc").cloned(),
        label: None,
        copyright: None,
        genres: Vec::new(),
        explicit: track.explicit,
        spotify_url: track.external_urls.get("spotify").cloned(),
        cover_url: image_url(&track.album.images),
        position,
    })
}

/// Builds a [`TrackMeta`] from an album page track (album metadata comes from
/// the parent [`FullAlbum`], which also supplies artist and cover fallbacks).
fn track_meta_from_simplified(
    track: &SimplifiedTrack,
    album: &FullAlbum,
    fallback_cover: Option<&str>,
) -> Option<TrackMeta> {
    let id = track.id.as_ref()?.id().to_string();

    let mut artists = artist_names(&track.artists);
    if artists.is_empty() {
        artists = artist_names(&album.artists);
    }

    let cover_url = image_url(&album.images).or_else(|| fallback_cover.map(str::to_string));

    let mut meta = TrackMeta {
        id,
        title: track.name.clone(),
        artists,
        album: album.name.clone(),
        album_id: Some(album.id.id().to_string()),
        album_artists: artist_names(&album.artists),
        track_number: track.track_number,
        total_tracks: Some(album.tracks.total),
        disc_number: disc_to_u32(track.disc_number),
        duration_ms: duration_ms(&track.duration),
        release_date: clean_date(Some(&album.release_date)),
        isrc: None,
        label: None,
        copyright: None,
        genres: Vec::new(),
        explicit: track.explicit,
        spotify_url: track.external_urls.get("spotify").cloned(),
        cover_url,
        position: 0,
    };
    apply_album_metadata(&mut meta, album, true);
    Some(meta)
}

fn artist_names(artists: &[SimplifiedArtist]) -> Vec<String> {
    artists.iter().map(|artist| artist.name.clone()).collect()
}

fn image_url(images: &[Image]) -> Option<String> {
    images.first().map(|image| image.url.clone())
}

fn duration_ms(duration: &chrono::Duration) -> u64 {
    duration.num_milliseconds().max(0) as u64
}

/// Spotify disc numbers are positive; clamp defensively instead of wrapping.
fn disc_to_u32(disc_number: i32) -> u32 {
    disc_number.max(0) as u32
}

/// Normalizes a Spotify release date (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`).
fn clean_date(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|date| !date.is_empty())
        .map(str::to_string)
}

/// Message shown when Spotify no longer accepts the stored session.
pub const SIGNED_OUT_MESSAGE: &str = "Spotify sign-in expired. Reconnect Spotify in Settings.";

/// True when an error message means the stored Spotify session is unusable, so
/// one token refresh (then a retry) is worth attempting.
pub fn is_sign_in_error(message: &str) -> bool {
    message == SIGNED_OUT_MESSAGE
}

fn spotify_error(action: &str, error: rspotify::ClientError) -> String {
    match error {
        rspotify::ClientError::InvalidToken => SIGNED_OUT_MESSAGE.to_string(),
        rspotify::ClientError::Http(http_error) => {
            if let rspotify::http::HttpError::StatusCode(response) = http_error.as_ref() {
                match response.status().as_u16() {
                    401 => {
                        return SIGNED_OUT_MESSAGE.to_string();
                    }
                    403 => {
                        return format!(
                            "Spotify refused access to this {action}. It may be private, \
                             region-restricted, or unavailable to this app."
                        );
                    }
                    404 => {
                        return format!("That {action} isn't available to this Spotify account.");
                    }
                    429 => {
                        return "Spotify's rate limit was reached. Try again in a few minutes."
                            .to_string();
                    }
                    _ => {}
                }
            }
            format!("Spotify rejected the {action} request: {http_error}")
        }
        other => format!("Spotify rejected the {action} request: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_track_url_with_query_string() {
        let (kind, id) = parse_spotify_url(
            "https://open.spotify.com/track/4iV5W9uYEdYUVa79Axb7Rh?si=d3adbe8e5c8f4c0f",
        )
        .expect("track URL should parse");

        assert_eq!(kind, CollectionKind::Track);
        assert_eq!(id, "4iV5W9uYEdYUVa79Axb7Rh");
    }

    #[test]
    fn parses_album_url_with_query_string() {
        let (kind, id) =
            parse_spotify_url("https://open.spotify.com/album/6IcGNaXFRf5Y1jc7QsE9O2?si=abc123")
                .expect("album URL should parse");

        assert_eq!(kind, CollectionKind::Album);
        assert_eq!(id, "6IcGNaXFRf5Y1jc7QsE9O2");
    }

    #[test]
    fn parses_playlist_url_with_query_string() {
        let (kind, id) =
            parse_spotify_url("https://open.spotify.com/playlist/59ZbFPES4DQwEjBpWHzrtC?si=xyz")
                .expect("playlist URL should parse");

        assert_eq!(kind, CollectionKind::Playlist);
        assert_eq!(id, "59ZbFPES4DQwEjBpWHzrtC");
    }

    #[test]
    fn parses_spotify_uri() {
        let (kind, id) = parse_spotify_url("  spotify:playlist:59ZbFPES4DQwEjBpWHzrtC  ")
            .expect("Spotify URI should parse");

        assert_eq!(kind, CollectionKind::Playlist);
        assert_eq!(id, "59ZbFPES4DQwEjBpWHzrtC");

        let (kind, id) = parse_spotify_url("spotify:track:4iV5W9uYEdYUVa79Axb7Rh")
            .expect("track URI should parse");
        assert_eq!(kind, CollectionKind::Track);
        assert_eq!(id, "4iV5W9uYEdYUVa79Axb7Rh");
    }

    #[test]
    fn parses_localized_intl_url() {
        let (kind, id) = parse_spotify_url(
            "https://open.spotify.com/intl-de/track/4iV5W9uYEdYUVa79Axb7Rh?si=abc",
        )
        .expect("localized URL should parse");

        assert_eq!(kind, CollectionKind::Track);
        assert_eq!(id, "4iV5W9uYEdYUVa79Axb7Rh");
    }

    #[test]
    fn rejects_artist_urls() {
        let error = parse_spotify_url("https://open.spotify.com/artist/2WX2uTcsvV5OnS0inACecP")
            .expect_err("artist URL should be rejected");

        assert!(error.to_lowercase().contains("artist"));
    }

    #[test]
    fn rejects_unsupported_and_garbage_input() {
        assert!(
            parse_spotify_url("https://open.spotify.com/episode/2WX2uTcsvV5OnS0inACecP").is_err()
        );
        assert!(parse_spotify_url("spotify:show:2WX2uTcsvV5OnS0inACecP").is_err());
        assert!(parse_spotify_url("https://example.com/not-a-track").is_err());
        assert!(parse_spotify_url("hello world").is_err());
        assert!(parse_spotify_url("   ").is_err());
        assert!(parse_spotify_url("https://open.spotify.com/track/short").is_err());
        // A 23-character ID must be rejected, not silently truncated.
        assert!(
            parse_spotify_url("https://open.spotify.com/track/4iV5W9uYEdYUVa79Axb7RhX").is_err()
        );
    }
}
