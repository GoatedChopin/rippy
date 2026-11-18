//! The update feed, `rippy-update.json`, published with every release. It holds a manifest (the
//! version and one download for each kind of computer) and an Ed25519 signature over the
//! manifest's exact text, so Rippy only installs builds that the release key signed.
//!
//! `examples/update_feed.rs` writes and signs feeds with this same code.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

/// The release key's public half, as base64. Its private half is "Rippy update key" in the
/// release Mac's login Keychain. A test build can swap in its own key at compile time with
/// `RIPPY_UPDATE_PUBLIC_KEY`, so test feeds are never signed with the real one.
pub const PUBLIC_KEY: &str = match option_env!("RIPPY_UPDATE_PUBLIC_KEY") {
    Some(key) => key,
    None => "XN4QQC/pZZ7JCQRMt+2vUWqe09vwTkPnU9KqetIZMrA=",
};

pub const APP: &str = "Rippy";

#[derive(Serialize, Deserialize)]
pub struct Feed {
    /// The manifest's JSON, exactly as it was signed.
    pub manifest: String,
    /// A base64 Ed25519 signature over `manifest`.
    pub signature: String,
}

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub app: String,
    pub version: String,
    /// One download per target; see [`target`].
    pub downloads: BTreeMap<String, Download>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Download {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// Every target a release can carry a download for.
#[allow(dead_code)] // Only examples/update_feed.rs reads it.
pub const TARGETS: [&str; 5] = ["macos", "windows-x64", "windows-arm64", "linux-x64", "linux-arm64"];

/// The download this computer needs. The macOS build is universal.
pub fn target() -> Option<&'static str> {
    let arm = cfg!(target_arch = "aarch64");
    if cfg!(target_os = "macos") {
        Some("macos")
    } else if cfg!(windows) {
        Some(if arm { "windows-arm64" } else { "windows-x64" })
    } else if cfg!(target_os = "linux") {
        Some(if arm { "linux-arm64" } else { "linux-x64" })
    } else {
        None
    }
}

/// A version is plain numbers and dots, like 0.3.0.
pub fn is_version(version: &str) -> bool {
    !version.is_empty()
        && version.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn base64_bytes<const N: usize>(text: &str, what: &str) -> Result<[u8; N]> {
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .with_context(|| format!("the {what} isn't base64"))?
        .try_into()
        .map_err(|_| anyhow!("the {what} isn't {N} bytes"))
}

impl Feed {
    /// The manifest, once its signature checks out against `public_key`.
    pub fn verify(&self, public_key: &str) -> Result<Manifest> {
        if public_key.is_empty() {
            bail!("this build has no update key");
        }
        let key = VerifyingKey::from_bytes(&base64_bytes(public_key, "update key")?)?;
        let signature = Signature::from_bytes(&base64_bytes(&self.signature, "signature")?);
        key.verify_strict(self.manifest.as_bytes(), &signature)
            .context("the update's signature doesn't match Rippy's key")?;

        let manifest: Manifest = serde_json::from_str(&self.manifest)?;
        if manifest.app != APP {
            bail!("the feed is for {}, not {APP}", manifest.app);
        }
        if !is_version(&manifest.version) {
            bail!("unexpected version '{}'", manifest.version);
        }
        Ok(manifest)
    }
}

/// Compares versions by their numbers, so 0.10.0 is newer than 0.9.1.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |version: &str| {
        version
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parse(candidate) > parse(current)
}
