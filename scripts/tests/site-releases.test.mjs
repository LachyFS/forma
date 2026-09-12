import assert from "node:assert/strict";
import { test } from "node:test";
import { macDownloads, describeRelease, fetchLatestRelease, RELEASE_API_URL } from "../../site/releases.mjs";

const base = "https://github.com/lachyfs/forma/releases";
const asset = (name, extra = {}) => ({
  name, browser_download_url: `${base}/download/v0.2.0/${name}`, size: 10 * 1024 * 1024,
  state: "uploaded", ...extra,
});
const release = (extra = {}) => ({
  tag_name: "v0.2.0", html_url: `${base}/tag/v0.2.0`, published_at: "2026-09-12T00:00:00Z",
  assets: [asset("Forma-macos-arm64.dmg")], ...extra,
});

test("offers Mac builds with explicit architectures, preferring universal", () => {
  const downloads = macDownloads([
    asset("Forma-x86_64-apple-darwin.zip"), asset("Forma-macos-arm64.dmg"),
    asset("Forma-universal.dmg"), asset("Forma.app.zip"),
  ]);
  assert.deepEqual(downloads.map((item) => item.architecture), ["Universal", "Apple silicon", "Intel", "macOS"]);
  assert.equal(downloads[0].label, "Universal · DMG · 10.0 MB");
});

test("does not offer Linux, Windows, source archives, checksums, debug symbols, or incomplete uploads", () => {
  const downloads = macDownloads([
    asset("Forma-linux-arm64.tar.gz"), asset("Forma-windows-x64.zip"), asset("Forma-macos-source.zip"),
    asset("Forma-macos-arm64.dmg.sha256"), asset("Forma-macos-arm64.dsym.zip"),
    asset("Forma-macos-arm64.dmg", { state: "new" }), asset("arbitrary.zip"), null,
  ]);
  assert.deepEqual(downloads, []);
  assert.deepEqual(macDownloads(null), []);
});

test("only links to HTTPS release assets belonging to this repository", () => {
  for (const url of [
    "javascript:alert(1)", "https://github.com.evil.test/lachyfs/forma/releases/download/v1/Forma.dmg",
    "https://github.com/another/project/releases/download/v1/Forma.dmg", "http://github.com/lachyfs/forma/releases/download/v1/Forma.dmg",
    "https://user:password@github.com/lachyfs/forma/releases/download/v1/Forma.dmg", "/relative/path",
  ]) assert.deepEqual(macDownloads([asset("Forma.dmg", { browser_download_url: url })]), []);
});

test("accepts a simple Mac app bundle and sorts formats consistently", () => {
  const downloads = macDownloads([asset("Forma.zip"), asset("Forma-universal.zip"), asset("Forma-universal.dmg"), asset("Forma-universal-2.dmg")]);
  assert.equal(downloads.length, 3);
  assert.equal(downloads[0].architecture, "Universal");
  assert.equal(downloads.at(-1).name, "Forma.zip");
});

test("accepts GitHub's canonical capitalization without changing asset names", () => {
  const url = "https://github.com/LachyFS/forma/releases/download/v0.2.0/Forma-macos-arm64.dmg";
  const downloads = macDownloads([asset("Forma-macos-arm64.dmg", { browser_download_url: url })]);
  assert.equal(downloads.length, 1);
  assert.equal(downloads[0].url, url);
  assert.equal(describeRelease(release({ html_url: "https://github.com/LachyFS/forma/releases/tag/v0.2.0" })).version, "v0.2.0");
});

test("handles source-only releases and rejects drafts, prereleases, and malformed data", () => {
  assert.deepEqual(describeRelease(release({ assets: [] })).downloads, []);
  assert.equal(describeRelease(release({ published_at: "invalid" })).date, "Latest stable release");
  for (const value of [null, {}, release({ draft: true }), release({ prerelease: true }), release({ html_url: "https://example.com" })]) {
    assert.throws(() => describeRelease(value));
  }
});

test("uses the latest stable release API without sending credentials", async () => {
  const result = await fetchLatestRelease(async (url, options) => {
    assert.equal(url, RELEASE_API_URL);
    assert.equal(options.credentials, "omit");
    assert.equal(options.referrerPolicy, "no-referrer");
    assert.ok(options.signal instanceof AbortSignal);
    return { ok: true, status: 200, json: async () => release() };
  });
  assert.equal(result.state, "available");
  assert.equal(result.release.version, "v0.2.0");
  assert.equal(result.release.downloads[0].architecture, "Apple silicon");
});

test("keeps missing, rate-limited, offline, and malformed responses recoverable", async () => {
  assert.deepEqual(await fetchLatestRelease(async () => ({ ok: false, status: 404 })), { state: "unavailable" });
  for (const fetcher of [
    async () => ({ ok: false, status: 403 }),
    async () => { throw new TypeError("Offline"); },
    async () => ({ ok: true, status: 200, json: async () => ({}) }),
    async () => ({ ok: true, status: 200, json: async () => { throw new Error("Invalid JSON"); } }),
  ]) assert.deepEqual(await fetchLatestRelease(fetcher), { state: "error" });
});
