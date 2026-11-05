use rustypipe::client::RustyPipe;
use rustypipe::model::{Thumbnail, VideoItem};
use serde::Serialize;

/// One search result, shared by the window and the API.
#[derive(Clone, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub url: String,
    pub title: String,
    pub channel: Option<String>,
    /// Seconds; live streams have none.
    pub duration: Option<u32>,
    pub thumbnail: Option<String>,
}

impl From<VideoItem> for SearchHit {
    fn from(item: VideoItem) -> Self {
        Self {
            url: format!("https://www.youtube.com/watch?v={}", item.id),
            thumbnail: thumbnail_url(&item.thumbnail),
            id: item.id,
            title: item.name,
            channel: item.channel.map(|channel| channel.name),
            duration: item.duration,
        }
    }
}

pub async fn search(youtube: &RustyPipe, query: &str) -> Result<Vec<SearchHit>, rustypipe::error::Error> {
    let found = youtube.query().search::<VideoItem, _>(query).await?;
    Ok(found.items.items.into_iter().map(SearchHit::from).collect())
}

/// The smallest thumbnail that still looks sharp at list size.
fn thumbnail_url(thumbnails: &[Thumbnail]) -> Option<String> {
    thumbnails
        .iter()
        .filter(|thumbnail| thumbnail.width >= 240)
        .min_by_key(|thumbnail| thumbnail.width)
        .or(thumbnails.last())
        .map(|thumbnail| thumbnail.url.clone())
}
