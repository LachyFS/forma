// Keep this repository aligned with the ordinary GitHub links in index.html.
export const REPOSITORY = "lachyfs/forma";
export const RELEASES_URL = `https://github.com/${REPOSITORY}/releases`;
export const LATEST_RELEASE_URL = `${RELEASES_URL}/latest`;
export const RELEASE_API_URL = `https://api.github.com/repos/${REPOSITORY}/releases/latest`;

function githubReleaseURL(value, kind) {
  try {
    const url = new URL(value);
    const prefix = `/${REPOSITORY}/releases/${kind}/`;
    return url.protocol === "https:" && url.hostname === "github.com"
      && !url.port && !url.username && !url.password
      && url.pathname.slice(0, prefix.length).toLowerCase() === prefix.toLowerCase()
      && url.pathname.length > prefix.length
      ? url.href : null;
  } catch {
    return null;
  }
}

export function formatSize(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function macDownloads(assets) {
  if (!Array.isArray(assets)) return [];
  return assets.flatMap((asset) => {
    if (!asset || typeof asset.name !== "string") return [];
    const name = asset.name;
    const url = githubReleaseURL(asset.browser_download_url, "download");
    const extension = name.match(/\.(dmg|pkg|zip|tar\.gz)$/i)?.[1]?.toLowerCase();
    if (!url || !extension || (asset.state && asset.state !== "uploaded")) return [];
    // Do not mistake Linux archives, checksums, debug symbols, or source for an app.
    if (/(?:linux|windows|win32|win64|mingw|msvc|gnu|android|source|symbols|debug|dsym)/i.test(name)) return [];
    const nativePackage = extension === "dmg" || extension === "pkg";
    const namedMac = /(?:macos|mac[-_.]|osx|darwin|\.app(?:\.|-))/i.test(name)
      || /^forma\.zip$/i.test(name);
    if (!nativePackage && !namedMac) return [];
    const silicon = /(?:aarch64|arm64|apple[-_. ]?silicon)/i.test(name);
    const intel = /(?:x86_64|x86-64|amd64|x64|intel)/i.test(name);
    const universal = /universal/i.test(name) || (silicon && intel);
    const architecture = universal ? "Universal" : silicon ? "Apple silicon" : intel ? "Intel" : "macOS";
    const rank = universal ? 0 : silicon ? 1 : intel ? 2 : 3;
    const size = formatSize(asset.size);
    return [{ name, url, architecture, size, rank, extension: extension.toUpperCase(), label: `${architecture} · ${extension.toUpperCase()}${size ? ` · ${size}` : ""}` }];
  }).sort((a, b) => a.rank - b.rank
    || Number(b.extension === "DMG") - Number(a.extension === "DMG")
    || a.name.localeCompare(b.name));
}

export function describeRelease(release) {
  if (!release || typeof release.tag_name !== "string" || !release.tag_name.trim()
    || release.draft || release.prerelease) throw new Error("No stable release");
  const url = githubReleaseURL(release.html_url, "tag");
  if (!url) throw new Error("Invalid release URL");
  const timestamp = release.published_at ? Date.parse(release.published_at) : NaN;
  return {
    version: release.tag_name,
    url,
    date: Number.isFinite(timestamp) ? new Intl.DateTimeFormat("en", {
      day: "numeric", month: "short", year: "numeric", timeZone: "UTC",
    }).format(timestamp) : "Latest stable release",
    downloads: macDownloads(release.assets),
  };
}

export async function fetchLatestRelease(fetcher = globalThis.fetch) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 8000);
  try {
    const response = await fetcher(RELEASE_API_URL, {
      headers: { Accept: "application/vnd.github+json" },
      signal: controller.signal,
      credentials: "omit",
      referrerPolicy: "no-referrer",
    });
    if (response.status === 404) return { state: "unavailable" };
    if (!response.ok) throw new Error("Release lookup failed");
    return { state: "available", release: describeRelease(await response.json()) };
  } catch {
    return { state: "error" };
  } finally {
    clearTimeout(timeout);
  }
}
