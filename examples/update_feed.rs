//! Makes and checks Rippy's update feed (`rippy-update.json`). `script/publish-release.sh` runs
//! it; the private key only ever arrives on stdin.
//!
//!   update_feed keygen                       prints a new private key
//!   update_feed public                       prints the public key for the private key on stdin
//!   update_feed sign <version> <base-url> <target>=<zip>...
//!                                            prints a feed signed with the private key on stdin
//!   update_feed verify <feed> [<target>=<zip>...]
//!                                            checks a feed against the key Rippy is built with

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest, Sha256};

#[path = "../src/feed.rs"]
#[allow(dead_code)]
mod feed;

use feed::{Download, Feed, Manifest};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => keygen(),
        Some("public") => {
            println!("{}", public_key(&read_key()?));
            Ok(())
        }
        Some("sign") if args.len() >= 4 => sign(&args[1], &args[2], &args[3..]),
        Some("verify") if args.len() >= 2 => verify(&args[1], &args[2..]),
        _ => bail!("usage: update_feed keygen | public | sign <version> <base-url> <target>=<zip>... | verify <feed> [<target>=<zip>...]"),
    }
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn keygen() -> Result<()> {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let mut out = std::io::stdout();
    writeln!(out, "{}", b64().encode(seed))?;
    Ok(())
}

fn read_key() -> Result<SigningKey> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let seed: [u8; 32] = b64()
        .decode(text.trim())
        .context("the private key isn't base64")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("the private key isn't 32 bytes"))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn public_key(key: &SigningKey) -> String {
    b64().encode(key.verifying_key().to_bytes())
}

/// `<target>=<zip>` pairs, with each zip's size and SHA-256.
fn downloads(base_url: Option<&str>, pairs: &[String]) -> Result<BTreeMap<String, (String, u64, String)>> {
    let mut downloads = BTreeMap::new();
    for pair in pairs {
        let (target, file) = pair
            .split_once('=')
            .with_context(|| format!("'{pair}' isn't <target>=<zip>"))?;
        if !feed::TARGETS.contains(&target) {
            bail!("unknown target '{target}'; Rippy knows {:?}", feed::TARGETS);
        }
        let bytes = std::fs::read(file).with_context(|| format!("couldn't read {file}"))?;
        let name = Path::new(file)
            .file_name()
            .context("no file name")?
            .to_string_lossy()
            .to_string();
        let url = base_url.map(|base| format!("{}/{name}", base.trim_end_matches('/')));
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        downloads.insert(target.to_string(), (url.unwrap_or(name), bytes.len() as u64, sha256));
    }
    Ok(downloads)
}

fn sign(version: &str, base_url: &str, pairs: &[String]) -> Result<()> {
    if !feed::is_version(version) {
        bail!("'{version}' isn't a version like 0.3.0");
    }
    if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        bail!("the base URL has to be http(s)");
    }
    let key = read_key()?;
    // A feed Rippy can't check is no use, so the key has to be the one it's built with.
    if public_key(&key) != feed::PUBLIC_KEY {
        bail!(
            "this private key isn't the one Rippy is built with (its public key is {}, Rippy's is '{}')",
            public_key(&key),
            feed::PUBLIC_KEY
        );
    }
    let manifest = Manifest {
        app: feed::APP.to_string(),
        version: version.to_string(),
        downloads: downloads(Some(base_url), pairs)?
            .into_iter()
            .map(|(target, (url, size, sha256))| (target, Download { url, size, sha256 }))
            .collect(),
    };
    let manifest = serde_json::to_string_pretty(&manifest)?;
    let signature = b64().encode(key.sign(manifest.as_bytes()).to_bytes());
    let feed = Feed { manifest, signature };
    feed.verify(feed::PUBLIC_KEY)?;
    println!("{}", serde_json::to_string_pretty(&feed)?);
    Ok(())
}

fn verify(file: &str, pairs: &[String]) -> Result<()> {
    let feed: Feed = serde_json::from_str(&std::fs::read_to_string(file)?)?;
    let manifest = feed.verify(feed::PUBLIC_KEY)?;
    println!("Signed by Rippy's key: {} {}", manifest.app, manifest.version);
    for (target, download) in &manifest.downloads {
        println!("  {target}: {} ({} bytes, {})", download.url, download.size, download.sha256);
    }
    for (target, (_, size, sha256)) in downloads(None, pairs)? {
        let listed = manifest
            .downloads
            .get(&target)
            .with_context(|| format!("the feed has no {target} download"))?;
        if listed.size != size || listed.sha256 != sha256 {
            bail!("the {target} zip doesn't match the feed");
        }
        println!("  {target} matches the zip");
    }
    Ok(())
}
