# Forma download site

A standalone static site: HTML, CSS, JavaScript, and local images. No framework,
package installation, build step, backend, analytics, or secrets are required.

## Preview

From the repository root:

```sh
python3 -m http.server 4173 --directory site
```

Open <http://localhost:4173>. Serve over HTTP rather than opening `index.html`
directly, because the release enhancements use JavaScript modules.

## GitHub Pages

The repository includes `.github/workflows/pages.yml`. It tests the release
logic and publishes `site/` directly, without bundling or a site build step.
The Quality workflow also runs syntax and download tests on every pull request.
Pages validation uses read-only permissions; a separate job publishes the
validated artifact. Manual deployment is restricted to `main`.

1. In the repository's **Settings → Pages**, set **Source** to **GitHub Actions**.
2. Merge the website and workflow into `main`. Changes under `site/`, the
   download tests, or the workflow trigger deployment automatically. You can
   also run **Deploy Forma website** manually from the Actions tab.
3. The deployment's URL is shown in the `github-pages` environment. For this
   repository's default project URL, it will be `https://lachyfs.github.io/forma/`.

The workflow uses GitHub's automatic token and Pages environment. No additional
secrets are needed. Node is used only to run the download checks; the published
site is plain files. See the official
[GitHub Pages workflow documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).

## Other static hosts

Publish the contents of `site/` with any static file host. Use `site` as the
publish directory and leave the build command empty. All local assets and
imports use relative paths, so the page also works under a repository subpath.
Serve `.mjs` files with a JavaScript MIME type.

The canonical and social metadata target `https://lachyfs.github.io/forma/`.
When publishing under another domain, update the canonical URL,
`og:url`, `og:image`, and `twitter:image` in `index.html` to that domain.
The social image is `assets/social.png`.
The custom social card is 1200 × 630. Do not use a local preview URL in public
metadata.

## Latest release downloads

`releases.mjs` requests the public GitHub API for the latest stable release of
`lachyfs/forma`. It reads the actual version, date, and assets, and offers direct
Mac downloads with explicit architecture labels. Multiple downloads appear in
a build selector; universal builds are listed first. No hardware architecture
is guessed from the browser's user agent.

For reliable recognition, name release assets clearly, for example:

- `Forma-macos-universal.dmg`
- `Forma-macos-arm64.dmg`
- `Forma-x86_64-apple-darwin.zip`
- `Forma.app.zip`

DMG and PKG packages are recognised as Mac packages. ZIP and TAR.GZ archives
need a Mac marker (`macos`, `mac-`, `osx`, `darwin`, `.app`) in their names;
the exact name `Forma.zip` is also accepted. Linux/Windows builds, source
archives, debug symbols, checksums, and incomplete uploads are excluded.
Only HTTPS asset URLs under this repository's GitHub release path are used.

If there is no accessible public release, the page says so. If GitHub is
offline or rate-limited, or JavaScript is disabled, ordinary GitHub release
links remain available. No release version, download size, or supported Mac
architecture is fabricated. The repository must be publicly accessible for
unauthenticated visitors to fetch releases. Never put a GitHub token in this
static site.

When moving repositories, update `REPOSITORY` in `releases.mjs` and the ordinary
GitHub links in `index.html` together. This site does not build or publish app
binaries; release packaging is a separate process.

## Validate download handling

```sh
node --test scripts/tests/site-releases.test.mjs
```

Tests run offline with Node 20 or later and require no dependencies. They cover
Mac asset selection, architecture labels, trusted download URLs, missing
releases, source-only releases, and network/API failures.

## Assets and accessibility

The visual layout is inspired by Zed's website: a dark slate palette, blue
controls, serif headings, compact navigation, and a grid of ruled sections.
The app icon and screenshots come from `assets/` and `docs/images/` in this
repository. WebP screenshots are resized for the page. `assets/social.png`
is a 1200 × 630 browser-rendered social card using the same typography,
colors, and actual Forma workspace image.

IBM Plex Sans, Serif, and Mono are self-hosted as compact Latin WOFF2 fonts.
Their SIL Open Font License files are included in `assets/fonts/`; the page
makes no font requests to an external service.

The page includes native FAQ disclosures, keyboard-operable screenshot
controls, visible focus states, a skip link, responsive layouts, and reduced
motion support. All primary navigation and release links work without
JavaScript.
