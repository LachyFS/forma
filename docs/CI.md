# Continuous integration

Pull requests, pushes to `main`, and merge queues run these checks. They have no
path filters, so documentation-only PRs also receive the status checks needed by
branch protection. New commits cancel superseded runs. Feature branches are
checked when a PR is opened, avoiding duplicate push and PR builds.

| Workflow | Checks |
| --- | --- |
| Quality | Rust formatting; actionlint validation of every workflow; ShellCheck on packaging scripts; JavaScript syntax and offline website tests. |
| Platforms | Strict Clippy on every workspace target, executable builds, core/application tests, and shader validation on Ubuntu 24.04, Windows 2025, and macOS 14. Linux also runs renderer tests serially through Mesa software Vulkan and renders all four viewport modes. |
| Security | Audits `Cargo.lock` against the current RustSec database on each change and daily at 03:23 UTC. No application build or GPU is needed. |
| Deploy Forma website | Tests and packages `site/`, then publishes to GitHub Pages after relevant changes on `main` or a manual run on `main`. |

Download `renderer-linux` from the Platforms run's artifacts to inspect smoke
images. They are retained for seven days; partial output is uploaded after a
failure when available. These are correctness checks, not performance benchmarks.
Hosted Windows/macOS jobs compile and link the application and translate shaders;
native Metal/DX12 execution and GUI interaction still need suitable machines as
described in [platform validation](PLATFORMS.md#validation-matrix).

## Toolchains and dependencies

`rust-toolchain.toml` pins Rust 1.92.0, including rustfmt and Clippy, for local
development and CI. Install [rustup](https://rust-lang.github.io/rustup/installation/index.html),
then run `rustup toolchain install` from the repository. Update the toolchain file
deliberately and validate all platforms when upgrading Rust. CI uses `--locked`
so dependency changes must include the resulting `Cargo.lock`.

Dependabot opens weekly updates for Cargo dependencies and GitHub Actions.
Compatible Cargo updates are grouped; major updates remain separate. Action
updates preserve full commit pins and their version comments. Updates need review
and passing checks before merging; there is no automatic merge workflow.

The security audit fails on known vulnerabilities. Informational advisories for
unmaintained crates remain visible as warnings, without an ignore list. The
initial audit found no known vulnerabilities but did report unmaintained
transitive dependencies including `async-std`, `instant`, `paste`,
`proc-macro-error2`, `rustls-pemfile`, `rustybuzz`, and `ttf-parser`. Resolving these
requires updates in their parent dependencies; warning-free dependency health is
not implied by a passing vulnerability audit.

The audit and lint tools are also versioned. When updating actionlint, update
both `ACTIONLINT_VERSION` and the Linux archive's `ACTIONLINT_SHA256` from the
upstream release checksums in `quality.yml`. Update the `shellcheck` and
`cargo-audit` tool versions in their install-action inputs as needed; Dependabot
updates the action itself, not these tool inputs or the Rust toolchain file.

## Permissions and repository settings

Checks use read-only repository permissions and do not retain checkout
credentials. External actions are pinned to full commit SHAs, following
[GitHub's secure-use guidance](https://docs.github.com/en/actions/reference/security/secure-use).
Only successful pushes to `main` save Rust build caches. The Pages publishing job
alone receives `pages: write` and `id-token: write`; it consumes the validated
artifact and does not check out or execute repository code.

To enforce checks before merging, configure a rule for `main` in repository
**Settings → Rules → Rulesets** after these workflows have run. Require:

- `Rust formatting`
- `Workflows and shell scripts`
- `Website tests`
- `Build and test (ubuntu-24.04)`
- `Build and test (windows-2025)`
- `Build and test (macos-14)`
- `Rust dependency audit`

Do not require the Pages deployment: it runs after a merge and only for relevant
paths. GitHub Pages must use **GitHub Actions** as its source. Workflow files do
not configure repository rulesets or Pages settings themselves.

## Run checks locally

Install the platform libraries listed in the root README, then run:

```sh
rustup toolchain install
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace --bins
cargo test --locked -p forma-core -p forma
node --check site/app.js
node --check site/releases.mjs
node --test scripts/tests/*.test.mjs
actionlint
shellcheck scripts/*.sh
cargo audit
```

Node 24 is used in CI. Install actionlint, ShellCheck, and cargo-audit separately
to run their checks locally. `cargo audit` reads the existing lockfile without
updating it.

For the same Linux software renderer used in CI:

```sh
export FORMA_RENDERER=vulkan
# Ubuntu 24.04; newer Mesa packages may name this lvp_icd.json.
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
export XDG_RUNTIME_DIR="$(mktemp -d)"
cargo test --locked -p forma-render -- --test-threads=1
cargo run --locked -p forma-render --bin render-smoke -- artifacts/render 4
rmdir "$XDG_RUNTIME_DIR"
```

Without a GPU or software Vulkan installation, shader translation can still run
with `cargo test --locked -p forma-render --test shaders`.
