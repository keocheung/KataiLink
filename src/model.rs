use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct MediaInfo {
    pub original_name: String,
    pub year: i32,
    pub tmdb_id: i64,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl MediaInfo {
    pub fn is_tv(&self) -> bool {
        self.season.is_some() && self.episode.is_some()
    }
}
