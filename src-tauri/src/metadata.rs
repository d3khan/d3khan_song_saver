use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CollectionKind {
    #[default]
    Track,
    Album,
    Playlist,
}

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
    pub release_date: Option<String>,
    pub isrc: Option<String>,
    pub label: Option<String>,
    pub copyright: Option<String>,
    pub genres: Vec<String>,
    pub explicit: bool,
    pub cover_url: Option<String>,
    pub position: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)] // ADDED SERDE DERIVES
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub year: Option<String>,
    pub duration_ms: u64,
}

pub fn parse_soundcloud_url(input: &str) -> Result<(CollectionKind, String), String> {
    if input.contains("/sets/") {
        Ok((CollectionKind::Playlist, input.to_string()))
    } else {
        Ok((CollectionKind::Track, input.to_string()))
    }
}
