//! Songsterr links to Guitar Pro files.
//!
//! Songsterr's web player reads these, with no sign-in:
//!
//! - `https://www.songsterr.com/api/meta/{song}[/{revision}]`: the song, its latest (or given)
//!   revision, tracks and YouTube videos
//! - `https://dqsljvtekg760.cloudfront.net/{song}/{revision}/{image}/{track}.json`: one track's
//!   notes, techniques, tempo map and lyrics
//! - `https://www.songsterr.com/api/video-points/{song}/{revision}/list`: for each video, the time
//!   in seconds where each bar starts
//!
//! The .gp is written by alphaTab (vendor/alphatab, the build Riffalong ships) running in QuickJS,
//! the JavaScript engine rustypipe already brings along. `songsterr/convert.js` builds the score,
//! writes it, reads the file back and compares every note.

use std::io::{Cursor, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use futures::{StreamExt as _, TryStreamExt as _};
use regex::Regex;
use rquickjs::{CatchResultExt as _, Context, Ctx, Function, Object, Runtime, TypedArray};
use serde::Serialize;
use serde_json::Value;

const API: &str = "https://www.songsterr.com";
const CDN: &str = "https://dqsljvtekg760.cloudfront.net";
/// Instrument id Songsterr gives drum tracks.
const DRUMS: u64 = 1024;
/// Track files fetched at once, about what Songsterr's own player asks for.
const PARALLEL_FETCHES: usize = 4;

const SHIMS: &str = include_str!("songsterr/shims.js");
const ALPHATAB: &str = include_str!("../vendor/alphatab/alphaTab.min.js");
const CONVERT: &str = include_str!("songsterr/convert.js");

/// The song id in a Songsterr link (`…-tab-s2`, `…-tab-s2t3`), or a bare id.
pub fn song_id(link: &str) -> Option<u64> {
    let link = link.trim();
    if let Ok(id) = link.parse::<u64>() {
        return (id > 0).then_some(id);
    }
    link_song_id(link)
}

/// The song id in a Songsterr link, with or without `https://`. Not a bare id, which could be a
/// search.
pub fn link_song_id(link: &str) -> Option<u64> {
    let link = link.trim();
    let url = reqwest::Url::parse(link)
        .ok()
        .filter(|url| url.has_host())
        .or_else(|| reqwest::Url::parse(&format!("https://{link}")).ok())?;
    let host = url.host_str()?;
    if host != "songsterr.com" && !host.ends_with(".songsterr.com") {
        return None;
    }
    static ID: OnceLock<Regex> = OnceLock::new();
    let id = ID.get_or_init(|| Regex::new(r"-s(\d+)(?:t\d+)?$").unwrap());
    let last = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
    id.captures(last)?[1].parse().ok().filter(|&id| id > 0)
}

/// What a song has, before converting it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub song_id: u64,
    pub revision_id: u64,
    pub title: String,
    pub artist: String,
    pub tracks: Vec<Track>,
    /// YouTube videos Songsterr lines the tab up with.
    pub videos: Vec<Video>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub name: String,
    pub instrument: String,
    /// MIDI note of each string, highest string first. Empty for drums.
    pub tuning: Vec<u64>,
    pub drums: bool,
    pub vocals: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    pub youtube_id: String,
    /// "alternative" (another recording), "backing", "solo", or none for the original.
    pub feature: Option<String>,
    /// For a solo video, the tracks it plays.
    pub tracks: Option<Vec<u64>>,
    /// The time in seconds where each bar starts in this video.
    pub bar_times: Vec<f64>,
}

/// A converted song.
pub struct Tab {
    pub file_name: String,
    pub gp: Vec<u8>,
    /// What convert.js counted after reading the .gp back.
    pub report: Value,
}

#[derive(Clone)]
pub struct Songsterr {
    client: reqwest::Client,
}

impl Songsterr {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Rippy/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    pub async fn song(&self, song: u64, revision: Option<u64>) -> anyhow::Result<Song> {
        let meta: Value = serde_json::from_str(&self.meta(song, revision).await?)?;
        let revision = number(&meta, "revisionId")?;
        // Some songs have no synced videos; the tab is still worth having.
        let points = match self
            .text(&format!("{API}/api/video-points/{song}/{revision}/list"))
            .await
        {
            Ok(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
            Err(err) => {
                eprintln!("Songsterr video points for {song}: {err}");
                Value::Null
            }
        };
        let tracks = meta["tracks"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|track| Track {
                name: string(track, "name"),
                instrument: string(track, "instrument"),
                tuning: track["tuning"]
                    .as_array()
                    .filter(|_| track["instrumentId"].as_u64() != Some(DRUMS))
                    .map(|tuning| tuning.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default(),
                drums: track["instrumentId"].as_u64() == Some(DRUMS),
                vocals: track["isVocalTrack"].as_bool().unwrap_or(false),
            })
            .collect();
        let mut videos: Vec<Video> = points
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|video| video["status"].as_str().is_none_or(|status| status == "done"))
            .filter_map(|video| {
                Some(Video {
                    youtube_id: video["videoId"].as_str()?.to_string(),
                    feature: video["feature"].as_str().map(String::from),
                    tracks: video["tracks"]
                        .as_array()
                        .map(|tracks| tracks.iter().filter_map(Value::as_u64).collect()),
                    bar_times: video["points"].as_array()?.iter().filter_map(Value::as_f64).collect(),
                })
            })
            .collect();
        // Songsterr's list can name the same video twice. It also stops at 52 entries.
        let mut seen = std::collections::HashSet::new();
        videos.retain(|video| seen.insert((video.youtube_id.clone(), video.feature.clone(), video.tracks.clone())));
        Ok(Song {
            song_id: song,
            revision_id: revision,
            title: string(&meta, "title"),
            artist: string(&meta, "artist"),
            tracks,
            videos,
        })
    }

    /// Fetches every track of a song and writes it as a Guitar Pro 7 file.
    pub async fn tab(&self, song: u64, revision: Option<u64>) -> anyhow::Result<Tab> {
        let meta_text = self.meta(song, revision).await?;
        let meta: Value = serde_json::from_str(&meta_text)?;
        let revision = number(&meta, "revisionId")?;
        let image = meta["image"]
            .as_str()
            .filter(|image| !image.is_empty())
            .ok_or_else(|| anyhow!("Songsterr has no tab for this song"))?;
        let track_count = meta["tracks"].as_array().map_or(0, Vec::len);
        if track_count == 0 {
            bail!("Songsterr has no tracks for this song");
        }
        let parts: Vec<String> = futures::stream::iter(0..track_count)
            .map(|track| self.text_owned(format!("{CDN}/{song}/{revision}/{image}/{track}.json")))
            .buffered(PARALLEL_FETCHES)
            .try_collect()
            .await?;
        let parts = format!("[{}]", parts.join(","));

        let (gp, report) = tokio::task::spawn_blocking(move || convert(&meta_text, &parts)).await??;
        let file_name = file_name(&format!("{} - {}", string(&meta, "artist"), string(&meta, "title")));
        Ok(Tab {
            file_name: format!("{file_name}.gp"),
            gp,
            report,
        })
    }

    async fn meta(&self, song: u64, revision: Option<u64>) -> anyhow::Result<String> {
        let url = match revision {
            Some(revision) => format!("{API}/api/meta/{song}/{revision}"),
            None => format!("{API}/api/meta/{song}"),
        };
        self.text(&url).await
    }

    async fn text_owned(&self, url: String) -> anyhow::Result<String> {
        self.text(&url).await
    }

    /// GETs a JSON document, trying again when Songsterr is busy.
    async fn text(&self, url: &str) -> anyhow::Result<String> {
        let mut attempt = 0;
        loop {
            let response = self
                .client
                .get(url)
                .header(reqwest::header::ACCEPT, "application/json")
                .send()
                .await;
            let retry = match response {
                Ok(response) if response.status().is_success() => return Ok(response.text().await?),
                Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                    bail!("Songsterr has no song or revision at {url}")
                }
                Ok(response)
                    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error() =>
                {
                    format!("Songsterr answered {}", response.status())
                }
                Ok(response) => bail!("Songsterr answered {} for {url}", response.status()),
                Err(err) if err.is_timeout() || err.is_connect() => err.to_string(),
                Err(err) => return Err(err.into()),
            };
            attempt += 1;
            if attempt == 3 {
                bail!("{retry} for {url}");
            }
            tokio::time::sleep(Duration::from_millis(1500 * attempt)).await;
        }
    }
}

fn number(value: &Value, key: &str) -> anyhow::Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| anyhow!("Songsterr's answer has no {key}"))
}

fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().trim().to_string()
}

/// A name every OS takes: no path or reserved characters, no trailing dots or spaces.
fn file_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let name = name.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let name: String = name.chars().take(150).collect();
    if name.is_empty() || name == "-" {
        "Songsterr tab".to_string()
    } else {
        name
    }
}

/// Writes the tab into `folder`, next to its final name first so a half-written file never has
/// it. A file already there with that name is replaced, as downloads are.
pub async fn save(tab: &Tab, folder: &Path) -> anyhow::Result<PathBuf> {
    let saved = async {
        tokio::fs::create_dir_all(folder).await?;
        let path = folder.join(&tab.file_name);
        let partial = folder.join(format!(".{}.part", tab.file_name));
        tokio::fs::write(&partial, &tab.gp).await?;
        if let Err(err) = tokio::fs::rename(&partial, &path).await {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(err);
        }
        Ok::<_, std::io::Error>(path)
    };
    saved
        .await
        .with_context(|| format!("Couldn't save {} in {}", tab.file_name, folder.display()))
}

/// One conversion at a time: a long song takes a few hundred MB while it's written and read back.
static CONVERTING: Mutex<()> = Mutex::new(());

/// Songsterr's meta and track files (a JSON array of them) to a .gp and convert.js's report.
pub fn convert(meta: &str, parts: &str) -> anyhow::Result<(Vec<u8>, Value)> {
    let _one_at_a_time = CONVERTING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime = Runtime::new()?;
    runtime.set_memory_limit(2 << 30);
    let context = Context::full(&runtime)?;
    let (stored, report) = context.with(|ctx| -> anyhow::Result<(Vec<u8>, String)> {
        let globals = ctx.globals();
        globals.set("__rippyUtf8Encode", Function::new(ctx.clone(), utf8_encode)?)?;
        globals.set("__rippyUtf8Decode", Function::new(ctx.clone(), utf8_decode)?)?;
        run(&ctx, SHIMS)?;
        run(&ctx, alphatab()?)?;
        run(&ctx, CONVERT)?;
        let songsterr_to_gp: Function = globals.get("songsterrToGp")?;
        let out: Object = songsterr_to_gp
            .call((meta, parts))
            .catch(&ctx)
            .map_err(|err| anyhow!("Couldn't convert the song: {err}"))?;
        let gp: TypedArray<u8> = out.get("gp")?;
        let gp = gp.as_bytes().context("alphaTab returned no file")?.to_vec();
        Ok((gp, out.get("report")?))
    })?;
    Ok((deflate(&stored)?, serde_json::from_str(&report)?))
}

fn run(ctx: &Ctx<'_>, source: &str) -> anyhow::Result<()> {
    ctx.eval::<(), _>(source)
        .catch(ctx)
        .map_err(|err| anyhow!("{err}"))
}

/// alphaTab, set to store the .gp's zip entries instead of deflating them: its deflate takes
/// seconds in QuickJS, so `deflate` does it in Rust.
fn alphatab() -> anyhow::Result<&'static str> {
    static PATCHED: OnceLock<Option<String>> = OnceLock::new();
    PATCHED
        .get_or_init(|| {
            let zip_entry = Regex::new(r"writeEntry\((\w+)\)\{const (\w+)=\w+\.CompressionMethodDeflate").unwrap();
            (zip_entry.find_iter(ALPHATAB).count() == 1)
                .then(|| zip_entry.replace(ALPHATAB, "writeEntry(${1}){const ${2}=0").into_owned())
        })
        .as_deref()
        .ok_or_else(|| anyhow!("alphaTab's zip writer changed; update the patch in songsterr.rs"))
}

fn utf8_encode<'js>(ctx: Ctx<'js>, text: String) -> rquickjs::Result<TypedArray<'js, u8>> {
    TypedArray::new(ctx, text.into_bytes())
}

fn utf8_decode(bytes: TypedArray<'_, u8>) -> String {
    let bytes = bytes.as_bytes().unwrap_or_default();
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// The same zip with its entries deflated, then checked entry by entry.
fn deflate(stored: &[u8]) -> anyhow::Result<Vec<u8>> {
    use zip::write::SimpleFileOptions;

    let mut archive = zip::ZipArchive::new(Cursor::new(stored))?;
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name()?.to_string();
        if entry.is_dir() {
            writer.add_directory(name, options)?;
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        writer.start_file(name.clone(), options)?;
        writer.write_all(&data)?;
        entries.push((name, data));
    }
    let deflated = writer.finish()?.into_inner();

    let mut check = zip::ZipArchive::new(Cursor::new(&deflated))?;
    for (name, data) in entries {
        let mut back = Vec::new();
        check.by_name(&name)?.read_to_end(&mut back)?;
        if back != data {
            bail!("{name} came out different after compressing the .gp");
        }
    }
    Ok(deflated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_song_id() {
        assert_eq!(song_id("https://www.songsterr.com/a/wsa/oasis-wonderwall-tab-s2"), Some(2));
        assert_eq!(song_id("https://www.songsterr.com/a/wsa/oasis-wonderwall-tab-s2t3"), Some(2));
        assert_eq!(
            song_id("https://www.songsterr.com/a/wsa/pink-floyd-comfortably-numb-tab-s271?x=1#top"),
            Some(271)
        );
        assert_eq!(song_id("songsterr.com/a/wsa/oasis-wonderwall-tab-s2"), Some(2));
        assert_eq!(song_id("www.songsterr.com/a/wsa/oasis-wonderwall-tab-s2"), Some(2));
        assert_eq!(song_id(" 271 "), Some(271));
        assert_eq!(link_song_id("271"), None);
        assert_eq!(link_song_id("oasis wonderwall"), None);
        assert_eq!(song_id("0"), None);
        assert_eq!(song_id("https://example.com/a/wsa/oasis-wonderwall-tab-s2"), None);
        assert_eq!(song_id("https://www.songsterr.com/a/wsa/oasis-wonderwall-tab"), None);
    }

    #[test]
    fn makes_safe_file_names() {
        assert_eq!(file_name("AC/DC - Back In Black"), "AC_DC - Back In Black");
        assert_eq!(file_name("Who? - What: \"Now\"..."), "Who_ - What_ _Now_");
        assert_eq!(file_name(" - "), "Songsterr tab");
    }

    #[test]
    fn patches_alphatab() {
        let uses = |source: &str| source.matches(".CompressionMethodDeflate").count();
        assert_eq!(uses(alphatab().unwrap()), uses(ALPHATAB) - 1);
    }

    /// A made-up two-track song in Songsterr's shape: guitar with techniques and lyrics, and drums.
    #[test]
    fn converts_a_song() {
        let meta = json!({
            "songId": 1, "revisionId": 5, "title": "Test Song", "artist": "Rippy", "image": "x",
            "tracks": [
                { "instrumentId": 25, "instrument": "Acoustic Guitar (steel)", "name": "Guitar" },
                { "instrumentId": 1024, "instrument": "Drums", "name": "Drums" },
            ],
        });
        let tuning = [64, 59, 55, 50, 45, 40];
        let guitar = json!({
            "instrumentId": 25, "name": "Guitar", "strings": 6, "tuning": tuning,
            "automations": { "tempo": [{ "measure": 0, "bpm": 96, "type": 4, "position": 0 }] },
            "newLyrics": [{ "line": 0, "offset": 1, "text": "Hel-lo world" }],
            "measures": [
                { "signature": [4, 4], "marker": { "text": "Intro" }, "clef": "F4", "voices": [{ "beats": [
                    { "type": 4, "duration": [1, 4], "chord": { "text": "Em7" }, "velocity": "mf",
                      "notes": [{ "string": 4, "fret": 2 }, { "string": 5, "fret": 0 }] },
                    { "type": 4, "duration": [1, 4], "notes": [{ "string": 1, "fret": 8, "hp": true }] },
                    { "type": 4, "duration": [1, 4], "notes": [{ "string": 1, "fret": 10,
                      "bend": { "points": [{ "position": 0, "tone": 0 }, { "position": 30, "tone": 100 }] } }] },
                    { "type": 8, "duration": [1, 12], "tuplet": 3, "notes": [{ "string": 0, "fret": 3, "slide": "shift" }] },
                    { "type": 8, "duration": [1, 12], "tuplet": 3, "notes": [{ "string": 0, "fret": 5 }] },
                    { "type": 8, "duration": [1, 12], "tuplet": 3, "rest": true },
                ]}]},
                { "voices": [{ "beats": [
                    { "type": 1, "duration": [1, 1], "notes": [{ "string": 2, "fret": 7, "tie": false, "vibrato": true }] },
                ]}]},
            ],
        });
        let drums = json!({
            "instrumentId": 1024, "name": "Drums",
            "measures": [
                { "voices": [{ "beats": [
                    { "type": 2, "duration": [1, 2], "notes": [{ "string": 5, "fret": 36 }, { "string": 0, "fret": 42 }] },
                    { "type": 2, "duration": [1, 2], "notes": [{ "string": 2, "fret": 38 }] },
                ]}]},
                { "voices": [{ "beats": [{ "type": 1, "duration": [1, 1], "rest": true }] }] },
            ],
        });
        let (gp, report) = convert(&meta.to_string(), &json!([guitar, drums]).to_string()).unwrap();

        assert_eq!(&gp[..2], b"PK");
        let mut zip = zip::ZipArchive::new(Cursor::new(&gp)).unwrap();
        let mut score = zip.by_name("Content/score.gpif").unwrap();
        assert_eq!(score.compression(), zip::CompressionMethod::Deflated);
        let mut gpif = String::new();
        score.read_to_string(&mut gpif).unwrap();
        // The guitar's bass clef in bar 1 holds through bar 2.
        assert_eq!(gpif.matches("<Clef>F4</Clef>").count(), 2);

        assert_eq!(report["title"], "Test Song");
        assert_eq!(report["bars"], 2);
        assert_eq!(report["notes"], 10);
        assert_eq!(report["matched"], 10);
        assert_eq!(report["lyrics"]["readBack"], 3); // Hel- lo world
        assert_eq!(report["sections"][0]["text"], "Intro");
        assert_eq!(report["barsNotFull"], 0);
        assert_eq!(report["notCarriedOver"], json!({}));
    }

    /// Fetches and converts Wonderwall from Songsterr: `cargo test -- --ignored songsterr`.
    #[tokio::test]
    #[ignore = "uses the network"]
    async fn songsterr_wonderwall() {
        let songsterr = Songsterr::new();
        let song = songsterr.song(2, None).await.unwrap();
        assert_eq!(song.title, "Wonderwall");
        assert!(!song.tracks.is_empty());
        let tab = songsterr.tab(2, None).await.unwrap();
        assert_eq!(tab.file_name, "Oasis - Wonderwall.gp");
        assert_eq!(tab.report["notes"], tab.report["matched"]);
    }
}
