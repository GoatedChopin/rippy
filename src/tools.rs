//! The command-line tools Rippy downloads with: yt-dlp, deno (the JavaScript runtime yt-dlp
//! needs for YouTube) and ffmpeg + ffprobe. Rippy installs its own copies on macOS and Windows,
//! checks for new releases at launch and once a day, and verifies every download's checksum.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use futures::StreamExt as _;
use gpui_kit::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const USER_AGENT: &str = "Rippy";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    YtDlp,
    Deno,
    Ffmpeg,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::YtDlp, Tool::Deno, Tool::Ffmpeg];

    pub fn name(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Deno => "deno",
            Tool::Ffmpeg => "ffmpeg",
        }
    }

    /// The executables a tool installs; the first is the one Rippy runs.
    fn binaries(self) -> &'static [&'static str] {
        match self {
            Tool::YtDlp => &["yt-dlp"],
            Tool::Deno => &["deno"],
            Tool::Ffmpeg => &["ffmpeg", "ffprobe"],
        }
    }

    /// Rippy's own copy, on platforms where it installs one.
    pub fn managed_path(self) -> Option<String> {
        Platform::current()?;
        Some(binary_path(self.binaries()[0]).to_string_lossy().to_string())
    }

    /// The command to run: Rippy's own copy, or the one on PATH elsewhere.
    pub fn command(self) -> String {
        self.managed_path().unwrap_or(self.name().to_string())
    }

    fn version_path(self) -> PathBuf {
        dir().join(format!("{}.version", self.name()))
    }

    pub fn installed_version(self) -> Option<String> {
        let version = std::fs::read_to_string(self.version_path()).ok()?;
        let complete = self.binaries().iter().all(|binary| binary_path(binary).is_file());
        complete.then(|| version.trim().to_string())
    }
}

/// How a version reads to people: ffmpeg's is "<build>_<version>", deno's starts with "v".
pub fn display_version(raw: &str) -> &str {
    raw.rsplit('_').next().unwrap_or(raw).trim_start_matches('v')
}

fn dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Rippy")
        .join("bin")
}

fn exe(binary: &str) -> String {
    format!("{binary}{}", std::env::consts::EXE_SUFFIX)
}

fn binary_path(binary: &str) -> PathBuf {
    dir().join(exe(binary))
}

/// What to download for this OS and CPU.
struct Platform {
    yt_dlp: &'static str,
    deno: &'static str,
    ffmpeg: FfmpegSource,
}

enum FfmpegSource {
    /// Martin Riedl's signed macOS builds; the value is the CPU name in his URLs.
    MartinRiedl(&'static str),
    /// gyan.dev's Windows release builds, linked from ffmpeg.org. x64 only; Windows on Arm emulates it.
    Gyan,
}

impl Platform {
    fn current() -> Option<Platform> {
        let arm = cfg!(target_arch = "aarch64");
        if cfg!(target_os = "macos") {
            Some(Platform {
                yt_dlp: "yt-dlp_macos",
                deno: if arm { "aarch64-apple-darwin" } else { "x86_64-apple-darwin" },
                ffmpeg: FfmpegSource::MartinRiedl(if arm { "arm64" } else { "amd64" }),
            })
        } else if cfg!(windows) {
            Some(Platform {
                yt_dlp: if arm { "yt-dlp_arm64.exe" } else { "yt-dlp.exe" },
                deno: if arm { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" },
                ffmpeg: FfmpegSource::Gyan,
            })
        } else {
            None
        }
    }
}

/// One release of a tool: its version and the files that make it up.
struct Release {
    version: String,
    downloads: Vec<Download>,
}

struct Download {
    url: String,
    sha256: String,
    /// A zip holding the executables, rather than the executable itself.
    zipped: bool,
}

#[derive(Clone)]
struct Clients {
    follow: reqwest::Client,
    no_follow: reqwest::Client,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

impl GithubRelease {
    fn asset(&self, name: &str) -> Result<&str> {
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
            .with_context(|| format!("the release has no {name}"))
    }
}

async fn text(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

async fn github_release(client: &reqwest::Client, repo: &str) -> Result<GithubRelease> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    Ok(serde_json::from_str(&text(client, &url).await?)?)
}

/// The SHA-256 in a checksum file, from the line naming `file` when the file lists several.
fn checksum(text: &str, file: Option<&str>) -> Result<String> {
    text.lines()
        .filter(|line| {
            file.is_none_or(|file| {
                line.split_whitespace()
                    .any(|word| word.trim_start_matches('*') == file)
            })
        })
        .flat_map(|line| line.split(|c: char| !c.is_ascii_hexdigit()))
        .find(|word| word.len() == 64)
        .map(str::to_lowercase)
        .context("no checksum for this download")
}

async fn latest(tool: Tool, platform: &Platform, clients: &Clients) -> Result<Release> {
    let client = &clients.follow;
    match tool {
        Tool::YtDlp => {
            let release = github_release(client, "yt-dlp/yt-dlp").await?;
            let sums = text(client, release.asset("SHA2-256SUMS")?).await?;
            Ok(Release {
                downloads: vec![Download {
                    url: release.asset(platform.yt_dlp)?.to_string(),
                    sha256: checksum(&sums, Some(platform.yt_dlp))?,
                    zipped: false,
                }],
                version: release.tag_name,
            })
        }
        Tool::Deno => {
            let release = github_release(client, "denoland/deno").await?;
            let zip = format!("deno-{}.zip", platform.deno);
            let sums = text(client, release.asset(&format!("{zip}.sha256sum"))?).await?;
            Ok(Release {
                downloads: vec![Download {
                    url: release.asset(&zip)?.to_string(),
                    sha256: checksum(&sums, None)?,
                    zipped: true,
                }],
                version: release.tag_name,
            })
        }
        Tool::Ffmpeg => match platform.ffmpeg {
            FfmpegSource::MartinRiedl(cpu) => {
                // The "latest" link redirects to the build's folder, which holds ffprobe too.
                let latest = format!(
                    "https://ffmpeg.martin-riedl.de/redirect/latest/macos/{cpu}/release/ffmpeg.zip"
                );
                let response = clients
                    .no_follow
                    .get(&latest)
                    .header("User-Agent", USER_AGENT)
                    .send()
                    .await?;
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .context("the latest ffmpeg link didn't redirect")?;
                let url = reqwest::Url::parse(&latest)?.join(location)?;
                let folder = url
                    .as_str()
                    .rsplit_once('/')
                    .map(|(folder, _)| folder.to_string())
                    .context("unexpected ffmpeg download address")?;
                let build = folder.rsplit('/').next().unwrap_or_default().to_string();

                let mut downloads = Vec::new();
                for file in ["ffmpeg.zip", "ffprobe.zip"] {
                    let url = format!("{folder}/{file}");
                    let sha256 = checksum(&text(client, &format!("{url}.sha256")).await?, None)?;
                    downloads.push(Download {
                        url,
                        sha256,
                        zipped: true,
                    });
                }
                Ok(Release {
                    version: build,
                    downloads,
                })
            }
            FfmpegSource::Gyan => {
                let builds = "https://www.gyan.dev/ffmpeg/builds";
                let version = text(client, &format!("{builds}/release-version")).await?;
                let version = version.trim().to_string();
                if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    bail!("unexpected ffmpeg version '{version}'");
                }
                let url = format!("{builds}/packages/ffmpeg-{version}-essentials_build.zip");
                let sha256 = checksum(&text(client, &format!("{url}.sha256")).await?, None)?;
                Ok(Release {
                    version,
                    downloads: vec![Download {
                        url,
                        sha256,
                        zipped: true,
                    }],
                })
            }
        },
    }
}

/// Versions compare by their numbers: 2026.10.26, v2.9.8, 1789931890_9.0.2.
fn is_newer(candidate: &str, installed: &str) -> bool {
    let parse = |version: &str| {
        version
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parse(candidate) > parse(installed)
}

/// Installs the latest release if it is newer than the installed one.
/// Returns the installed version and whether it was just replaced.
async fn update(tool: Tool, clients: Clients) -> Result<(String, bool)> {
    let platform = Platform::current().context("Rippy doesn't install tools on this platform")?;
    let release = latest(tool, &platform, &clients).await?;
    if let Some(installed) = tool.installed_version() {
        if !is_newer(&release.version, &installed) {
            return Ok((installed, false));
        }
    }

    // Stage everything beside the live copies and verify it before replacing anything.
    tokio::fs::create_dir_all(dir()).await?;
    remove_leftovers(tool);
    let mut staged = Vec::new();
    for (index, download) in release.downloads.iter().enumerate() {
        let file = dir().join(format!(".{}-{index}.download", tool.name()));
        fetch_verified(&clients.follow, download, &file).await?;
        if download.zipped {
            let wanted = tool.binaries().iter().map(|binary| exe(binary)).collect();
            let zip = file.clone();
            staged.extend(tokio::task::spawn_blocking(move || extract(&zip, wanted)).await??);
            let _ = tokio::fs::remove_file(&file).await;
        } else {
            staged.push((file, binary_path(tool.binaries()[0])));
        }
    }
    for binary in tool.binaries() {
        if !staged.iter().any(|(_, target)| *target == binary_path(binary)) {
            bail!("the {} download didn't include {}", tool.name(), exe(binary));
        }
    }

    for (file, target) in &staged {
        make_executable(file)?;
        replace(file, target)?;
    }
    tokio::fs::write(tool.version_path(), &release.version).await?;
    Ok((release.version, true))
}

async fn fetch_verified(client: &reqwest::Client, download: &Download, file: &Path) -> Result<()> {
    let mut out = tokio::fs::File::create(file).await?;
    let mut hasher = Sha256::new();
    let mut stream = client
        .get(&download.url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        out.write_all(&chunk).await?;
    }
    out.sync_all().await?;
    if format!("{:x}", hasher.finalize()) != download.sha256 {
        bail!("{} didn't match its checksum", download.url);
    }
    Ok(())
}

/// Pulls the wanted executables out of a zip, wherever they sit inside it.
fn extract(zip: &Path, wanted: Vec<String>) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip)?)?;
    let mut staged = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry
            .enclosed_name()
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
        else {
            continue;
        };
        if !wanted.contains(&name) {
            continue;
        }
        let file = dir().join(format!(".{name}.staged"));
        let mut out = std::fs::File::create(&file)?;
        io::copy(&mut entry, &mut out)?;
        out.sync_all()?;
        staged.push((file, dir().join(&name)));
    }
    Ok(staged)
}

fn make_executable(file: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

/// Windows can't overwrite a running program but can rename it, so move the old copy aside first.
fn replace(staged: &Path, target: &Path) -> io::Result<()> {
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    if target.exists() {
        std::fs::rename(target, &old)?;
    }
    std::fs::rename(staged, target)?;
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// Files an interrupted update left behind, and old copies Windows was still running last time.
fn remove_leftovers(tool: Tool) {
    for binary in tool.binaries() {
        let _ = std::fs::remove_file(dir().join(format!(".{}.staged", exe(binary))));
        let _ = std::fs::remove_file(binary_path(binary).with_extension("old"));
    }
    for index in 0..2 {
        let _ = std::fs::remove_file(dir().join(format!(".{}-{index}.download", tool.name())));
    }
}

pub enum Status {
    Checking,
    UpToDate,
    Updated,
    Failed,
}

struct ToolState {
    version: Option<SharedString>,
    status: Status,
    task: Option<Task<()>>,
}

/// Keeps every tool installed and current, checking at launch and once a day.
pub struct Tools {
    states: Vec<ToolState>,
    tokio: tokio::runtime::Handle,
    clients: Clients,
    _schedule: Task<()>,
}

impl Tools {
    pub fn new(tokio: tokio::runtime::Handle, cx: &mut Context<Self>) -> Self {
        let schedule = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.check(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            }
        });

        Self {
            states: Tool::ALL
                .iter()
                .map(|tool| ToolState {
                    version: tool.installed_version().map(SharedString::from),
                    status: Status::Checking,
                    task: None,
                })
                .collect(),
            tokio,
            clients: Clients {
                follow: reqwest::Client::new(),
                no_follow: reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .unwrap_or_default(),
            },
            _schedule: schedule,
        }
    }

    fn state(&self, tool: Tool) -> &ToolState {
        &self.states[tool as usize]
    }

    pub fn version(&self, tool: Tool) -> Option<&str> {
        self.state(tool)
            .version
            .as_deref()
            .map(display_version)
    }

    pub fn status(&self, tool: Tool) -> &Status {
        &self.state(tool).status
    }

    /// Downloads can run: every tool is installed, or this platform uses the ones on PATH.
    pub fn ready(&self) -> bool {
        Platform::current().is_none() || self.states.iter().all(|state| state.version.is_some())
    }

    pub fn checking(&self) -> bool {
        self.states.iter().any(|state| state.task.is_some())
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        for tool in Tool::ALL {
            let state = &mut self.states[tool as usize];
            if state.task.is_some() {
                continue;
            }
            state.status = Status::Checking;
            let job = self.tokio.spawn(update(tool, self.clients.clone()));
            state.task = Some(cx.spawn(async move |this, cx| {
                let result = job.await;
                this.update(cx, |this, cx| {
                    let state = &mut this.states[tool as usize];
                    state.task = None;
                    match result {
                        Ok(Ok((version, replaced))) => {
                            state.status = if replaced {
                                Status::Updated
                            } else {
                                Status::UpToDate
                            };
                            state.version = Some(version.into());
                        }
                        Ok(Err(err)) => {
                            eprintln!("{} update failed: {err:#}", tool.name());
                            state.status = Status::Failed;
                        }
                        Err(err) => {
                            eprintln!("{} update task failed: {err}", tool.name());
                            state.status = Status::Failed;
                        }
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }
}
