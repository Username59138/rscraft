use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha1::{Digest, Sha1};

const VERSION_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Deserialize)]
struct Manifest {
    versions: Vec<ManifestEntry>,
}

#[derive(Deserialize)]
struct ManifestEntry {
    id: String,
    url: String,
}

#[derive(Deserialize)]
struct VersionInfo {
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    client: Download,
}

#[derive(Deserialize)]
struct Download {
    url: String,
    sha1: String,
    size: u64,
}

fn cache_dir() -> Result<PathBuf> {
    Ok(dirs::data_dir().context("no data directory on this system")?.join("rscraft"))
}

/// Returns the path of the client jar for `version`, downloading it from
/// Mojang on first use. Checked in order: our cache, an existing Minecraft
/// installation, Mojang's servers.
pub fn ensure_client_jar(version: &str) -> Result<PathBuf> {
    let cached = cache_dir()?.join("versions").join(version).join("client.jar");
    if cached.is_file() {
        return Ok(cached);
    }

    if let Some(home) = dirs::home_dir() {
        let installed = home.join(".minecraft/versions").join(version).join(format!("{version}.jar"));
        if installed.is_file() {
            log::info!("using installed Minecraft jar {}", installed.display());
            return Ok(installed);
        }
    }

    download_client_jar(version, &cached)?;
    Ok(cached)
}

fn download_client_jar(version: &str, dest: &Path) -> Result<()> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).build().into();

    let manifest: Manifest = serde_json::from_str(&agent.get(VERSION_MANIFEST).call()?.body_mut().read_to_string()?)?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == version)
        .with_context(|| format!("Minecraft version {version} not found in Mojang's manifest"))?;
    let info: VersionInfo = serde_json::from_str(&agent.get(&entry.url).call()?.body_mut().read_to_string()?)?;
    let client = info.downloads.client;

    log::info!("downloading Minecraft {version} client jar ({:.1} MB)…", client.size as f64 / 1e6);
    fs::create_dir_all(dest.parent().unwrap())?;
    let partial = dest.with_extension("jar.part");

    let mut response = agent.get(&client.url).call()?;
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(&partial)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.sync_all()?;

    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if actual != client.sha1 {
        let _ = fs::remove_file(&partial);
        bail!("client jar checksum mismatch: expected {}, got {actual}", client.sha1);
    }
    fs::rename(&partial, dest)?;
    log::info!("saved {}", dest.display());
    Ok(())
}
