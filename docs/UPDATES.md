# Automatic updates

Packaged Forma releases check GitHub Releases 30 seconds after launch and roughly
every six hours. A worker downloads a full update in the background. The footer
shows **Update ready**; installation starts only when the user chooses
**Restart to update**. The application menu and command palette also expose
**Check for Updates** and update preferences. Automatic checks and downloads can
be disabled independently, and a version can be skipped. A manual check overrides
the skipped version. Preferences live in `updates.json` beside the theme setting.

Save, Discard, and Cancel are offered for unsaved projects. A save must succeed
before restart. Active saves, imports, exports, texture loads and shader work
block restart, and edits are locked while saving for an update. Project identity
and edit generation are checked again after asynchronous work. The helper waits
for Forma to exit, replaces the installation, then opens the saved project using
the same renderer selection. A failed save leaves the editor open.

## Packaging and authenticity

Forma uses [Velopack](https://docs.velopack.io/) 1.2.161 for native packaging,
download caching, application replacement and relaunch. A full package includes
the executable, update helper and denoiser runtime. macOS ships an application
ZIP, Windows a per-user installer, and Linux a writable AppImage. Existing builds
without this packaging require one manual installation. Development builds,
standalone binaries and sessions with `FORMA_DISABLE_UPDATES` set do not update.
Package managers should set that environment variable.

GitHub's release listing discovers versions; it is not the trust authority.
Each platform has an Ed25519-signed manifest. Forma authenticates the exact
payload bytes with an embedded public key before reading package details. It
checks repository, platform, channel, package identity, version, full-package
type, filename, size and SHA-256. Package bytes are checked after downloading and
again before handing them to the native helper. Missing metadata, bad signatures,
wrong architecture, corrupt packages, and downgrades fail closed. HTTPS requests
have timeouts; metadata reads have a size limit. Velopack's automatic startup
installation is disabled so it cannot bypass the verification and save flow.

Stable installations consume only stable releases. Preview installations consume
only prereleases; switching channels requires a manual installation. Complete
semantic versions, including `beta.N` and `rc.N`, are embedded from the tag.
Discovery considers the most recent 50 GitHub releases. There are no delta
updates, staged rollouts, forced restarts, or crash-health rollback in this first
implementation. Full packages cost more bandwidth but update companion libraries
together and simplify verification and recovery.

The release workflow builds and tests all four platforms before assembly. Only
assembly receives `FORMA_UPDATE_SIGNING_KEY`. It validates every package, signs
the manifests, verifies the signatures and hashes, and uploads all assets before
publishing the draft. Signing or packaging failure prevents publication. OS code
signing is separate: update authenticity works without paid Apple/Windows
accounts, but unsigned/ad-hoc initial installs can still display OS warnings.

## Failure and recovery

- Offline, rate-limited, or timed-out checks leave the running application alone.
  Open Updates for the error and retry with **Check now**. Automatic checks retry
  at the next scheduled interval.
- A failed or interrupted download never triggers restart. Retrying downloads
  again or reuses a verified cache. Corrupt cached packages cannot be installed.
- Insufficient disk space, locked files, read-only installations and permissions
  can make native replacement fail. Keep the application in a writable location,
  close other running copies, free space, then retry. Native helper logs contain
  replacement errors; if Forma cannot start, reinstall from GitHub Releases.
- A bad release should be superseded by a higher version containing the fix or
  reverted code. Automatic downgrades are rejected. A manual reinstall remains
  the recovery path; projects and preferences are outside the application bundle.
  Future document migrations must preserve compatibility or provide backups.
- Keep an offline backup of the signing key. Rotation needs a migration release
  signed by the old key, support for the new key, and a discovery plan that also
  reaches users who skip that migration release. This single-key implementation
  does not yet automate rotation; simply changing the key strands old installs.
  Losing the only key requires a manual reinstall. A compromised key requires
  incident recovery, not a silent key replacement in CI.

## Verification

`cargo test -p forma-update` covers signatures and tampering, channel/version and
package rejection, truncated or modified downloads, persisted preferences, and
restart rejection after failed saves or changed documents. Python release tests
exercise real OpenSSL signatures and fail incomplete or mismatched asset sets.
The native editor smoke test exercises modal input blocking, export guards,
successful saves before installation, and preservation of dirty work after a
failed save. Late file-dialog results are rejected while restart is pending.
The Update installation workflow packages versions A and B on each native target,
installs A, verifies and applies B through the real helper, and checks the
restarted executable and companion runtime are both B. These fixtures validate
both stable and preview upgrades. They validate
native replacement; they do not establish GPU compatibility or remove the need
to test native file pickers and first-install OS warnings on end-user machines.
