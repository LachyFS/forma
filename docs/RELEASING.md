# Releasing Forma

The [Release workflow](../.github/workflows/release.yml) builds native optimized
packages for macOS Apple silicon, macOS Intel, Windows x64, and Linux x86_64.
Every package includes the pinned, checksum-verified Open Image Denoise runtime.
The release commit must pass the same RustSec dependency audit used by branch CI
before any packages are built. All four builds and their core/application/shader
tests must also succeed before publication. Generated GitHub release notes include
merged pull requests and installation instructions. Packages are not Developer ID signed, notarized, or
Authenticode signed; macOS bundles receive an ad-hoc signature.

## Test the pipeline

Run **Actions → Release → Run workflow** on your branch, or:

```sh
gh workflow run release.yml --ref YOUR_BRANCH
```

Manual runs never create a tag or GitHub release. Download `release-complete`
from the run to inspect the four installable packages, four full update packages,
four signed manifests and `SHA256SUMS`. Install and launch
the packages on the appropriate machines before cutting a release. Hosted tests
do not establish native GUI or GPU compatibility. Linux packages target Ubuntu
24.04 or compatible newer systems and still require system desktop libraries.

## Publish a version

1. Set `[workspace.package].version` in `Cargo.toml` to the new numeric version
   (for example `0.2.0`). Run `cargo check --workspace` to update workspace versions
   in `Cargo.lock`, commit both files, and merge after the normal CI checks pass.
2. From that reviewed commit, create and push an annotated tag:

   ```sh
   git tag -a v0.2.0 -m 'Forma 0.2.0'
   git push origin v0.2.0
   ```

3. Follow the Release workflow. It verifies the tag's version matches the
   workspace, tests/builds/packages all four platforms, generates `SHA256SUMS`,
   creates a draft release, uploads every asset, then publishes it. The download
   website discovers the latest stable release automatically; no site deploy is
   needed.

For a preview use `v0.2.0-alpha.1`, `v0.2.0-beta.1`, or `v0.2.0-rc.1` against
workspace version `0.2.0`. These publish as prereleases and stay out of the
website's latest stable downloads. Each tag is a distinct release; never move a
published tag. The workflow does not create tags, bump versions, or publish crates.

If a build fails, fix it and use a new tag, or rerun for a transient failure.
A failed upload leaves a draft that a rerun can complete. Already published
releases are never overwritten by the workflow. Uploads finish before publishing,
which also supports repositories with immutable releases enabled.

## Verify downloads

Download `SHA256SUMS` beside the archives, then run:

```sh
sha256sum --check SHA256SUMS  # Linux
shasum -a 256 --check SHA256SUMS  # macOS
```

On Windows use `Get-FileHash .\Forma-v0.2.0-windows-x64.exe -Algorithm SHA256`
and compare with its entry in `SHA256SUMS`. Asset names include the tag, OS,
and processor. macOS uses an application ZIP, Windows a per-user installer,
and Linux an AppImage. Keep the macOS application bundle intact. The `.nupkg`
files and signed `update-*.json` manifests are updater inputs.

Only the final assembly/publication job has `contents: write`; package builds
have read-only repository access. The assembly job requires the repository secret
`FORMA_UPDATE_SIGNING_KEY`, an Ed25519 private key in PEM format. Its public key
must match `crates/forma-update/update-public-key.txt`; the signer rejects a
mismatch. A key and secret are configured for this repository. Back up the
private key securely: GitHub does not allow retrieving secret values. No personal
access token is required in Actions. Protect `v*` tags with a repository ruleset to restrict who can
release. Normal branch CI and tag protection are repository settings; the release
workflow does not enforce that a tag's commit has already passed branch CI.

The update signature is separate from paid OS code signing. Without Apple or
Windows credentials, the pipeline preserves ad-hoc macOS signatures and unsigned
Windows installers. Initial installation can show OS trust warnings. To add
platform signing later, provision certificates in the appropriate build job and
set `FORMA_MAC_SIGN_IDENTITY` (and optionally `FORMA_MAC_INSTALL_IDENTITY` and
`FORMA_MAC_NOTARY_PROFILE`), or `FORMA_WINDOWS_SIGN_PARAMS`. These are packager
hooks; certificate provisioning is not configured in this repository.

See [update design and recovery](UPDATES.md) for channels, key rotation, rollback,
and native upgrade tests. The release version is embedded from the complete tag,
including preview suffixes; a local build without `FORMA_RELEASE_VERSION` cannot
offer updates.
