//! Authenticated full-package updates. No GUI or installation policy lives here.
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    time::Duration,
};
use velopack::locator::{LocationContext, auto_locate_app_manifest};
use velopack::{UpdateInfo, UpdateManager, UpdateOptions, VelopackAsset, sources::HttpSource};

pub const REPOSITORY: &str = "lachyfs/forma";
pub const PACKAGE_ID: &str = "Forma";
pub const PUBLIC_KEY: &str = include_str!("../update-public-key.txt");
const MAX_METADATA: u64 = 2 * 1024 * 1024;
const MAX_PACKAGE: u64 = 2 * 1024 * 1024 * 1024;

pub fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x86_64",
        ("windows", "x86_64") => "windows-x64",
        ("linux", "x86_64") => "linux-x86_64",
        _ => "unsupported",
    }
}

pub fn channel(version: &Version) -> &'static str {
    if version.pre.is_empty() {
        "stable"
    } else {
        "preview"
    }
}

/// Velopack's implicit startup installation would bypass our signed metadata and
/// editor save flow. Every installation must instead pass through Candidate.
pub fn startup() {
    velopack::VelopackApp::build()
        .set_auto_apply_on_startup(false)
        .run();
}

fn manager(base: &str) -> Result<UpdateManager> {
    Ok(UpdateManager::new(
        HttpSource::new_with_options(
            base,
            velopack::HttpOptions {
                TimeoutMilliseconds: 300_000,
                ..Default::default()
            },
        ),
        Some(UpdateOptions {
            MaximumDeltasBeforeFallback: -1,
            ..Default::default()
        }),
        None,
    )?)
}

pub fn installed_version() -> Result<Version> {
    ensure!(
        !cfg!(debug_assertions),
        "Updates are available in packaged release builds."
    );
    ensure!(
        std::env::var_os("FORMA_DISABLE_UPDATES").is_none(),
        "Updates are managed outside Forma."
    );
    let embedded = option_env!("FORMA_RELEASE_VERSION")
        .context("Install a packaged release to receive updates.")?;
    let version = Version::parse(embedded)?;
    let manager = manager("https://github.com/lachyfs/forma/releases")?;
    ensure!(
        manager.get_current_version() == version,
        "Installed and application versions disagree."
    );
    let locator = auto_locate_app_manifest(LocationContext::FromCurrentExe)?;
    ensure!(
        locator.get_manifest_id() == PACKAGE_ID,
        "Unexpected installation identity."
    );
    ensure!(
        locator.get_manifest_channel() == format!("{}-{}", channel(&version), platform()),
        "Unexpected update channel."
    );
    Ok(version)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub payload: String,
    pub signature: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub repository: String,
    pub platform: String,
    pub channel: String,
    pub version: String,
    pub asset: VelopackAsset,
}

/// Authenticate the exact encoded bytes before interpreting any package fields.
pub fn verify_manifest(
    bytes: &[u8],
    key: &str,
    expected_platform: &str,
    current: &Version,
) -> Result<Manifest> {
    ensure!(
        bytes.len() as u64 <= MAX_METADATA,
        "Update metadata is too large"
    );
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    let payload = STANDARD.decode(envelope.payload)?;
    let signature = Signature::from_slice(&STANDARD.decode(envelope.signature)?)?;
    let key: [u8; 32] = STANDARD
        .decode(key.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid update public key"))?;
    VerifyingKey::from_bytes(&key)?
        .verify_strict(&payload, &signature)
        .context("Update signature is invalid")?;
    let manifest: Manifest = serde_json::from_slice(&payload)?;
    ensure!(
        manifest.schema == 1 && manifest.repository == REPOSITORY,
        "Unsupported update source"
    );
    ensure!(
        manifest.platform == expected_platform,
        "Update targets another platform"
    );
    ensure!(
        manifest.channel == channel(current),
        "Update targets another channel"
    );
    let version = Version::parse(&manifest.version)?;
    ensure!(
        channel(&version) == manifest.channel && version > *current,
        "Update is not a newer version in this channel"
    );
    ensure!(
        version.build.is_empty(),
        "Release versions must not contain build metadata"
    );
    let asset = &manifest.asset;
    ensure!(
        asset.PackageId == PACKAGE_ID && asset.Version == manifest.version && asset.Type == "Full",
        "Unexpected update package"
    );
    ensure!(
        !asset.FileName.is_empty()
            && asset
                .FileName
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._".contains(&c))
            && asset.FileName.ends_with("-full.nupkg")
            && !asset.FileName.contains(".."),
        "Invalid update filename"
    );
    ensure!(
        asset.Size > 0 && asset.Size <= MAX_PACKAGE,
        "Invalid update size"
    );
    ensure!(
        asset.SHA256.len() == 64 && asset.SHA256.bytes().all(|c| c.is_ascii_hexdigit()),
        "Update has no valid SHA-256 digest"
    );
    Ok(manifest)
}

fn metadata(client: &Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send()?.error_for_status()?;
    let mut bytes = Vec::new();
    response.take(MAX_METADATA + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_METADATA,
        "Update response is too large"
    );
    Ok(bytes)
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

pub struct Candidate {
    manager: UpdateManager,
    manifest: Manifest,
}

impl Candidate {
    pub fn version(&self) -> &str {
        &self.manifest.version
    }

    pub fn check() -> Result<Option<Self>> {
        let current = installed_version()?;
        let client = Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("Forma-Updater")
            .build()?;
        let releases: Vec<Release> = serde_json::from_slice(&metadata(
            &client,
            &format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=50"),
        )?)?;
        let latest = releases
            .into_iter()
            .filter(|r| !r.draft && r.prerelease != current.pre.is_empty())
            .filter_map(|r| {
                r.tag_name
                    .strip_prefix('v')
                    .and_then(|tag| Version::parse(tag).ok())
            })
            .filter(|v| v > &current && v.build.is_empty() && channel(v) == channel(&current))
            .max();
        let Some(version) = latest else {
            return Ok(None);
        };
        let base = format!("https://github.com/{REPOSITORY}/releases/download/v{version}");
        let manifest = verify_manifest(
            &metadata(&client, &format!("{base}/update-{}.json", platform()))?,
            PUBLIC_KEY,
            platform(),
            &current,
        )?;
        ensure!(
            manifest.version == version.to_string(),
            "Release and signed metadata disagree"
        );
        Ok(Some(Self {
            manager: manager(&base)?,
            manifest,
        }))
    }

    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            TargetFullRelease: self.manifest.asset.clone(),
            ..Default::default()
        }
    }

    pub fn download(&self, progress: Sender<i16>) -> Result<()> {
        // Velopack returns early for an existing cache entry without hashing it.
        // Remove invalid entries so a retry can actually fetch a fresh package.
        clear_invalid_cache(&self.package_path()?, &self.manifest.asset)?;
        self.manager
            .download_updates(&self.info(), Some(progress))?;
        self.verify_download()
    }

    fn package_path(&self) -> Result<PathBuf> {
        Ok(auto_locate_app_manifest(LocationContext::FromCurrentExe)?
            .get_packages_dir()
            .join(&self.manifest.asset.FileName))
    }

    fn verify_download(&self) -> Result<()> {
        verify_package(&self.package_path()?, &self.manifest.asset)
    }

    /// Must be called only after the editor has completed saving and locked input.
    pub fn install_after_exit(&self, args: Vec<String>) -> Result<()> {
        self.verify_download()?;
        self.manager
            .wait_exit_then_apply_updates(self.info(), false, true, args)?;
        Ok(())
    }
}

fn clear_invalid_cache(path: &Path, asset: &VelopackAsset) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    if verify_package(path, asset).is_err() {
        // Remove only this cache entry, including a symlink itself. Never recurse
        // into directories or follow a link to delete its destination.
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Could not clear the invalid cached update"),
        }
    }
    Ok(())
}

pub fn verify_package(path: &Path, asset: &VelopackAsset) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() == asset.Size,
        "Downloaded update has the wrong size or type"
    );
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    ensure!(
        format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&asset.SHA256),
        "Downloaded update failed verification"
    );
    Ok(())
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Preferences {
    pub automatic_checks: bool,
    pub automatic_downloads: bool,
    pub skipped_version: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic_checks: true,
            automatic_downloads: true,
            skipped_version: None,
        }
    }
}

impl Preferences {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("No preference directory")?;
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut temp, self)?;
        temp.as_file().sync_all()?;
        temp.persist(path)?;
        Ok(())
    }
}

/// Restart authorization is invalidated by edits or a document switch during a save.
pub fn may_restart(
    saved: bool,
    dirty: bool,
    busy: bool,
    original: (u64, u64),
    current: (u64, u64),
) -> bool {
    saved && !dirty && !busy && original == current
}

#[cfg(test)]
mod tests;
