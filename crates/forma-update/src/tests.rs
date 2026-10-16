use super::*;
use ed25519_dalek::{Signer, SigningKey};

fn manifest() -> Manifest {
    Manifest {
        schema: 1,
        repository: REPOSITORY.into(),
        platform: "linux-x86_64".into(),
        channel: "stable".into(),
        version: "1.2.0".into(),
        asset: VelopackAsset {
            PackageId: PACKAGE_ID.into(),
            Version: "1.2.0".into(),
            Type: "Full".into(),
            FileName: "Forma-1.2.0-full.nupkg".into(),
            Size: 3,
            SHA256: format!("{:x}", Sha256::digest(b"abc")),
            ..Default::default()
        },
    }
}

fn signed(manifest: &Manifest) -> (Vec<u8>, String) {
    let key = SigningKey::from_bytes(&[17; 32]);
    let payload = serde_json::to_vec(manifest).unwrap();
    let envelope = Envelope {
        signature: STANDARD.encode(key.sign(&payload).to_bytes()),
        payload: STANDARD.encode(payload),
    };
    (
        serde_json::to_vec(&envelope).unwrap(),
        STANDARD.encode(key.verifying_key().as_bytes()),
    )
}

fn verify(bytes: &[u8], key: &str) -> Result<Manifest> {
    verify_manifest(
        bytes,
        key,
        "linux-x86_64",
        &Version::parse("1.0.0").unwrap(),
    )
}

#[test]
fn valid_release_and_tampering() {
    let (bytes, key) = signed(&manifest());
    assert_eq!(verify(&bytes, &key).unwrap().version, "1.2.0");
    assert!(verify(&bytes, &STANDARD.encode([1; 32])).is_err());
    let mut envelope: Envelope = serde_json::from_slice(&bytes).unwrap();
    let mut payload = STANDARD.decode(&envelope.payload).unwrap();
    payload[4] ^= 1;
    envelope.payload = STANDARD.encode(payload);
    assert!(verify(&serde_json::to_vec(&envelope).unwrap(), &key).is_err());
}

#[test]
fn signed_but_inapplicable_packages_are_rejected() {
    for change in [
        |m: &mut Manifest| m.platform = "macos-arm64".into(),
        |m: &mut Manifest| m.channel = "preview".into(),
        |m: &mut Manifest| m.version = "0.9.0".into(),
        |m: &mut Manifest| m.asset.Version = "3.0.0".into(),
        |m: &mut Manifest| m.asset.PackageId = "OtherApp".into(),
        |m: &mut Manifest| m.asset.Type = "Delta".into(),
        |m: &mut Manifest| m.asset.FileName = "../bad-full.nupkg".into(),
        |m: &mut Manifest| m.asset.FileName = "https://bad/full.nupkg".into(),
        |m: &mut Manifest| m.asset.SHA256.clear(),
        |m: &mut Manifest| m.asset.Size = 0,
        |m: &mut Manifest| m.asset.Size = MAX_PACKAGE + 1,
        |m: &mut Manifest| m.repository = "attacker/repo".into(),
        |m: &mut Manifest| m.schema = 2,
    ] {
        let mut m = manifest();
        change(&mut m);
        let (bytes, key) = signed(&m);
        assert!(verify(&bytes, &key).is_err(), "accepted {m:?}");
    }
}

#[test]
fn preview_versions_are_distinct_and_channels_never_cross() {
    let mut m = manifest();
    m.channel = "preview".into();
    m.version = "1.2.0-beta.2".into();
    m.asset.Version = m.version.clone();
    let (bytes, key) = signed(&m);
    assert!(
        verify_manifest(
            &bytes,
            &key,
            "linux-x86_64",
            &Version::parse("1.2.0-beta.1").unwrap()
        )
        .is_ok()
    );
    assert!(
        verify_manifest(
            &bytes,
            &key,
            "linux-x86_64",
            &Version::parse("1.2.0-beta.2").unwrap()
        )
        .is_err()
    );
    assert!(verify(&bytes, &key).is_err());
}

#[test]
fn interrupted_or_modified_download_cannot_install() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("update");
    fs::write(&path, b"abc").unwrap();
    assert!(verify_package(&path, &manifest().asset).is_ok());
    fs::write(&path, b"ab").unwrap();
    assert!(verify_package(&path, &manifest().asset).is_err());
    fs::write(&path, b"abd").unwrap();
    assert!(verify_package(&path, &manifest().asset).is_err());
}

#[test]
fn retry_removes_invalid_cache_but_keeps_verified_packages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("update");
    let asset = manifest().asset;
    clear_invalid_cache(&path, &asset).unwrap();
    for bytes in [b"ab".as_slice(), b"abd".as_slice()] {
        fs::write(&path, bytes).unwrap();
        clear_invalid_cache(&path, &asset).unwrap();
        assert!(!path.exists());
    }
    fs::write(&path, b"abc").unwrap();
    clear_invalid_cache(&path, &asset).unwrap();
    verify_package(&path, &asset).unwrap();
    #[cfg(unix)]
    {
        let link = dir.path().join("linked-update");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        clear_invalid_cache(&link, &asset).unwrap();
        assert!(fs::symlink_metadata(link).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"abc");
    }
    let directory = dir.path().join("directory");
    fs::create_dir(&directory).unwrap();
    assert!(clear_invalid_cache(&directory, &asset).is_err());
    assert!(directory.is_dir());
}

#[test]
fn failed_save_new_edits_new_document_or_active_export_cancel_restart() {
    assert!(may_restart(true, false, false, (1, 2), (1, 2)));
    assert!(!may_restart(false, false, false, (1, 2), (1, 2)));
    assert!(!may_restart(true, true, false, (1, 2), (1, 2)));
    assert!(!may_restart(true, false, true, (1, 2), (1, 2)));
    assert!(!may_restart(true, false, false, (1, 2), (1, 3)));
    assert!(!may_restart(true, false, false, (1, 2), (2, 2)));
}

#[test]
fn preferences_persist_and_corruption_is_not_silently_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config/updates.json");
    assert!(Preferences::load(&path).unwrap().automatic_downloads);
    let prefs = Preferences {
        automatic_checks: false,
        automatic_downloads: false,
        skipped_version: Some("1.2.0".into()),
    };
    prefs.save(&path).unwrap();
    prefs.save(&path).unwrap();
    assert_eq!(
        Preferences::load(&path).unwrap().skipped_version,
        prefs.skipped_version
    );
    fs::write(path.clone(), b"broken").unwrap();
    assert!(Preferences::load(&path).is_err());
}
