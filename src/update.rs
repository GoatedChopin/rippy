//! Rippy keeps itself up to date. Shortly after launch, and then once a day, it reads the feed
//! on GitHub's latest release and checks the feed's signature. If there's a newer version, it
//! downloads this computer's build, checks its size and SHA-256, and puts it where the installed
//! copy is. The running copy keeps going, and the new version starts with the next launch, or
//! straight away from **Restart to update**.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use futures::StreamExt as _;
use gpui_kit::*;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::feed::{self, Feed};

const FEED: &str =
    "https://github.com/GoatedChopin/rippy/releases/latest/download/rippy-update.json";
const FIRST_CHECK: Duration = Duration::from_secs(5);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// No Rippy build comes close to this, so anything bigger isn't one.
const MAX_SIZE: u64 = 512 * 1024 * 1024;
const USER_AGENT: &str = concat!("Rippy/", env!("CARGO_PKG_VERSION"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set on the copy that Restart launches, so it waits for the old copy to let go of the API port.
pub const RELAUNCHED: &str = "RIPPY_RELAUNCHED";

/// What an update replaces.
#[derive(Clone)]
enum Install {
    /// The executable itself, on Windows and Linux.
    Executable(PathBuf),
    /// The whole Rippy.app, on macOS.
    Bundle(PathBuf),
}

impl Install {
    fn folder(&self) -> &Path {
        let (Install::Executable(path) | Install::Bundle(path)) = self;
        path.parent().unwrap_or(Path::new("."))
    }

    fn name(&self) -> String {
        let (Install::Executable(path) | Install::Bundle(path)) = self;
        path.file_name().unwrap_or_default().to_string_lossy().to_string()
    }

    /// The copy an update moved aside. It goes at the next launch, once nothing is running it.
    fn old(&self) -> PathBuf {
        let name = self.name();
        self.folder().join(match self {
            Install::Executable(_) => format!("{name}.old"),
            Install::Bundle(_) => format!(".{name}.old"),
        })
    }

    fn staging(&self) -> PathBuf {
        self.folder().join(format!(".{}.update", self.name()))
    }
}

/// The feed to read. Debug builds only update when a test feed is set.
fn feed_url() -> Option<String> {
    match std::env::var("RIPPY_UPDATE_FEED") {
        Ok(url) if !url.is_empty() => Some(url),
        _ if cfg!(debug_assertions) => None,
        _ => Some(FEED.to_string()),
    }
}

fn locate() -> Result<Install, &'static str> {
    let exe = std::env::current_exe().map_err(|_| "Rippy can't tell where it's installed")?;
    if cfg!(target_os = "macos") {
        let exe = exe.canonicalize().unwrap_or(exe);
        let bundle = exe
            .ancestors()
            .nth(3)
            .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
            .ok_or("Updates come with the downloaded app")?;
        // macOS runs an app opened straight from a download out of a read-only copy.
        if bundle.to_string_lossy().contains("/AppTranslocation/") {
            return Err("Move Rippy to Applications to get updates");
        }
        Ok(Install::Bundle(bundle.to_path_buf()))
    } else {
        Ok(Install::Executable(exe))
    }
}

fn cache() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Rippy")
        .join("update")
}

/// The copy an earlier update moved aside, and anything an interrupted one left.
fn remove_leftovers(install: &Install) {
    let old = install.old();
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_file(install.staging());
    let _ = std::fs::remove_dir_all(install.staging());
    let _ = std::fs::remove_dir_all(cache());
}

/// The newer release's download for this computer, if there is one.
async fn find(client: reqwest::Client, url: String) -> Result<Option<(String, feed::Download)>> {
    let text = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let feed: Feed = serde_json::from_str(&text).context("the update feed isn't readable")?;
    let manifest = feed.verify(feed::PUBLIC_KEY)?;
    if !feed::is_newer(&manifest.version, VERSION) {
        return Ok(None);
    }
    let target = feed::target().context("there are no Rippy builds for this computer")?;
    let download = manifest
        .downloads
        .get(target)
        .cloned()
        .with_context(|| format!("Rippy {} has no {target} build", manifest.version))?;
    Ok(Some((manifest.version, download)))
}

/// Downloads and checks the build, then puts it in place of this copy.
async fn download_and_apply(
    client: reqwest::Client,
    download: feed::Download,
    install: Install,
) -> Result<()> {
    if download.size > MAX_SIZE {
        bail!("the update is {} bytes, far too big for Rippy", download.size);
    }
    // A test feed may be served locally; the real one is only ever HTTPS.
    let testing = std::env::var_os("RIPPY_UPDATE_FEED").is_some();
    if !(download.url.starts_with("https://") || testing && download.url.starts_with("http://")) {
        bail!("the update isn't served over HTTPS");
    }

    let folder = cache();
    tokio::fs::create_dir_all(&folder).await?;
    let zip = folder.join("rippy-update.zip");
    let fetched = fetch_verified(&client, &download, &zip).await;
    let applied = match fetched {
        Ok(()) => {
            let zip = zip.clone();
            tokio::task::spawn_blocking(move || apply(&zip, &install)).await?
        }
        Err(err) => Err(err),
    };
    let _ = tokio::fs::remove_dir_all(&folder).await;
    applied
}

async fn fetch_verified(client: &reqwest::Client, download: &feed::Download, file: &Path) -> Result<()> {
    let response = client
        .get(&download.url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?;
    let mut out = tokio::fs::File::create(file).await?;
    let mut hasher = Sha256::new();
    let mut size = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size += chunk.len() as u64;
        if size > download.size {
            bail!("the update is bigger than its feed says");
        }
        hasher.update(&chunk);
        out.write_all(&chunk).await?;
    }
    out.sync_all().await?;
    if size != download.size {
        bail!("the update is {size} bytes, but its feed says {}", download.size);
    }
    if format!("{:x}", hasher.finalize()) != download.sha256.to_lowercase() {
        bail!("the update didn't match its checksum");
    }
    Ok(())
}

fn apply(zip: &Path, install: &Install) -> Result<()> {
    remove_staging(install);
    let applied = (|| match install {
        Install::Executable(exe) => {
            let wanted = if cfg!(windows) { "Rippy.exe" } else { "rippy" };
            let staged = install.staging();
            extract_one(zip, wanted, &staged)?;
            make_executable(&staged)?;
            swap(&staged, exe, &install.old())
        }
        Install::Bundle(app) => {
            let staging = install.staging();
            std::fs::create_dir(&staging)?;
            // ditto keeps the bundle's symlinks, permissions and signature intact.
            run(Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(zip).arg(&staging))?;
            let staged = staging.join("Rippy.app");
            if !staged.is_dir() {
                bail!("the update has no Rippy.app");
            }
            check_signature(app, &staged)?;
            swap(&staged, app, &install.old())
        }
    })();
    // Whether it worked or not, nothing should stay unpacked beside the installed copy.
    remove_staging(install);
    applied
}

/// What an update unpacked beside the installed copy. (The cache folder is still in use.)
fn remove_staging(install: &Install) {
    let staging = install.staging();
    let _ = std::fs::remove_file(&staging);
    let _ = std::fs::remove_dir_all(&staging);
}

fn extract_one(zip: &Path, wanted: &str, to: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry
            .enclosed_name()
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_string()));
        if entry.is_file() && name.as_deref() == Some(wanted) {
            let mut out = std::fs::File::create(to)?;
            io::copy(&mut entry, &mut out)?;
            out.sync_all()?;
            return Ok(());
        }
    }
    bail!("the update has no {wanted}")
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

/// Moves the installed copy aside and the new one into its place. Windows can't overwrite a
/// running program but can rename it, and a virus scan can hold a new file for a moment.
fn swap(staged: &Path, target: &Path, old: &Path) -> Result<()> {
    let _ = std::fs::remove_file(old);
    let _ = std::fs::remove_dir_all(old);
    retry(|| std::fs::rename(target, old)).context("couldn't move the installed copy aside")?;
    if let Err(err) = retry(|| std::fs::rename(staged, target)) {
        let _ = std::fs::rename(old, target);
        return Err(err).context("couldn't put the new copy in place");
    }
    Ok(())
}

fn retry(mut step: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    for _ in 0..9 {
        match step() {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(err),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    step()
}

fn run(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        bail!(
            "{:?} failed: {}",
            command.get_program(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    // codesign writes what it shows to stderr.
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// The new app must be whole, and signed by whoever signed this one.
fn check_signature(installed: &Path, staged: &Path) -> Result<()> {
    run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(staged))
        .context("the update's code signature doesn't check out")?;
    let signer = |app: &Path| -> Result<(String, Option<String>)> {
        let details = run(Command::new("/usr/bin/codesign").arg("-dv").arg(app))?;
        let field = |name: &str| {
            details
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .map(str::to_string)
        };
        let identifier = field("Identifier=").context("the app has no signing identifier")?;
        let team = field("TeamIdentifier=").filter(|team| team != "not set");
        Ok((identifier, team))
    };
    let (identifier, team) = signer(installed)?;
    let (new_identifier, new_team) = signer(staged)?;
    if new_identifier != identifier {
        bail!("the update is {new_identifier}, not {identifier}");
    }
    if team.is_some() && new_team != team {
        bail!("the update isn't signed by the same developer");
    }
    Ok(())
}

/// What went wrong, in words for Settings; the details go to the log.
fn problem(err: &anyhow::Error) -> &'static str {
    let denied = err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|err| err.kind() == io::ErrorKind::PermissionDenied)
    });
    if denied {
        "Rippy can't update itself in the folder it's in"
    } else {
        "Couldn't install the update"
    }
}

#[derive(Clone)]
pub enum Status {
    Checking,
    UpToDate,
    Downloading(SharedString),
    /// This version is installed and starts with the next launch.
    Ready(SharedString),
    Failed(&'static str),
    /// This copy doesn't update itself, for the reason given.
    Unavailable(&'static str),
}

pub struct Updater {
    status: Status,
    install: Option<Install>,
    feed: Option<String>,
    tokio: tokio::runtime::Handle,
    client: reqwest::Client,
    task: Option<Task<()>>,
    _schedule: Option<Task<()>>,
}

impl Updater {
    pub fn new(tokio: tokio::runtime::Handle, cx: &mut Context<Self>) -> Self {
        let install = locate();
        let feed = feed_url();
        let status = match (&install, &feed) {
            (Err(reason), _) => Status::Unavailable(*reason),
            (_, None) => Status::Unavailable("Updates come with the downloaded app"),
            _ if feed::PUBLIC_KEY.is_empty() => Status::Unavailable("This build doesn't update itself"),
            _ => Status::Checking,
        };
        let available = matches!(status, Status::Checking);
        let schedule = available.then(|| {
            cx.spawn(async move |this, cx| {
                // Give a copy that Restart replaced a moment to quit, so its leftovers can go.
                cx.background_executor().timer(FIRST_CHECK).await;
                loop {
                    if this.update(cx, |this, cx| this.check(cx)).is_err() {
                        break;
                    }
                    cx.background_executor().timer(CHECK_EVERY).await;
                }
            })
        });

        Self {
            status,
            install: install.ok(),
            feed,
            tokio,
            client: reqwest::Client::new(),
            task: None,
            _schedule: schedule,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn checking(&self) -> bool {
        self.task.is_some()
    }

    pub fn ready(&self) -> Option<&SharedString> {
        match &self.status {
            Status::Ready(version) => Some(version),
            _ => None,
        }
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        // Once an update is in place, the next one waits for a restart.
        if self.task.is_some() || matches!(self.status, Status::Ready(_) | Status::Unavailable(_)) {
            return;
        }
        let (Some(install), Some(feed)) = (self.install.clone(), self.feed.clone()) else {
            return;
        };
        remove_leftovers(&install);
        self.status = Status::Checking;
        let client = self.client.clone();
        let tokio = self.tokio.clone();
        let found = tokio.spawn(find(client.clone(), feed));

        self.task = Some(cx.spawn(async move |this, cx| {
            let (version, download) = match found.await {
                Ok(Ok(Some(found))) => found,
                Ok(Ok(None)) => {
                    this.update(cx, |this, cx| this.finish(Status::UpToDate, cx)).ok();
                    return;
                }
                Ok(Err(err)) => {
                    eprintln!("Couldn't check for a Rippy update: {err:#}");
                    let status = Status::Failed("Couldn't check for updates");
                    this.update(cx, |this, cx| this.finish(status, cx)).ok();
                    return;
                }
                Err(err) => {
                    eprintln!("Rippy update check failed: {err}");
                    let status = Status::Failed("Couldn't check for updates");
                    this.update(cx, |this, cx| this.finish(status, cx)).ok();
                    return;
                }
            };

            let version = SharedString::from(version);
            let installing = tokio.spawn(download_and_apply(client, download, install));
            this.update(cx, |this, cx| {
                this.status = Status::Downloading(version.clone());
                cx.notify();
            })
            .ok();
            let status = match installing.await {
                Ok(Ok(())) => Status::Ready(version),
                Ok(Err(err)) => {
                    eprintln!("Couldn't install Rippy {version}: {err:#}");
                    Status::Failed(problem(&err))
                }
                Err(err) => {
                    eprintln!("Rippy update failed: {err}");
                    Status::Failed("Couldn't install the update")
                }
            };
            this.update(cx, |this, cx| this.finish(status, cx)).ok();
        }));
        cx.notify();
    }

    fn finish(&mut self, status: Status, cx: &mut Context<Self>) {
        self.status = status;
        self.task = None;
        cx.notify();
    }

    /// Starts the new version. The caller quits this one once it has started.
    pub fn relaunch(&self) -> io::Result<()> {
        let Some(install) = &self.install else {
            return Err(io::Error::other("Rippy doesn't know where it's installed"));
        };
        match install {
            Install::Bundle(app) => Command::new("/usr/bin/open")
                .arg("-n")
                .arg("--env")
                .arg(format!("{RELAUNCHED}=1"))
                .arg(app)
                .spawn()
                .map(drop),
            Install::Executable(exe) => Command::new(exe).env(RELAUNCHED, "1").spawn().map(drop),
        }
    }
}
