//! Tiny installed application used by the native A -> B upgrade smoke test.
use anyhow::{Context, Result};
use std::{fs, path::PathBuf};
use velopack::{
    UpdateInfo, UpdateManager,
    locator::{LocationContext, auto_locate_app_manifest},
    sources::FileSource,
};

fn main() -> Result<()> {
    forma_update::startup();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--write-version") => {
            let sibling = std::env::current_exe()?
                .parent()
                .context("executable directory")?
                .join("oidn/fixture-version");
            fs::write(
                &args[1],
                format!(
                    "{}\n{}",
                    option_env!("FIXTURE_VERSION").unwrap_or("1.0.0"),
                    fs::read_to_string(sibling)?
                ),
            )?;
        }
        Some("--update") => {
            let source = PathBuf::from(&args[1]);
            let locator = auto_locate_app_manifest(LocationContext::FromCurrentExe)?;
            let manifest = forma_update::verify_manifest(
                &fs::read(source.join(format!("update-{}.json", forma_update::platform())))?,
                &fs::read_to_string(&args[2])?,
                forma_update::platform(),
                &locator.get_manifest_version(),
            )?;
            let manager = UpdateManager::new(FileSource::new(&source), None, None)?;
            let update = UpdateInfo {
                TargetFullRelease: manifest.asset,
                ..Default::default()
            };
            manager.download_updates(&update, None)?;
            forma_update::verify_package(
                &locator
                    .get_packages_dir()
                    .join(&update.TargetFullRelease.FileName),
                &update.TargetFullRelease,
            )?;
            manager.wait_exit_then_apply_updates(
                update,
                true,
                true,
                vec!["--write-version".to_owned(), args[3].clone()],
            )?;
        }
        _ => {}
    }
    Ok(())
}
