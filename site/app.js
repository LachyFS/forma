// Progressive enhancements. All primary links also work without JavaScript.
import { fetchLatestRelease, RELEASES_URL } from "./releases.mjs";

const comparison = document.querySelector(".comparison");
const comparisonRange = document.querySelector("#comparison-range");

function updateComparison() {
  const shaded = Number(comparisonRange.value);
  comparison.style.setProperty("--split", `${shaded}%`);
  comparisonRange.setAttribute("aria-valuetext", `${shaded}% shaded, ${100 - shaded}% wireframe`);
}

comparisonRange.addEventListener("input", updateComparison);
comparisonRange.hidden = false;
document.querySelector("#comparison-help").textContent = "Drag the divider or use the arrow keys to compare.";
updateComparison();

const downloadLink = document.querySelector("#release-download");
const downloadLabel = document.querySelector("#download-label");
const downloadStatus = document.querySelector("#download-status");

function selectDownload(download) {
  downloadLink.href = download.url;
  document.querySelector(".hero-download").href = download.url;
  downloadLabel.textContent = `Download for ${download.architecture === "Universal" ? "Mac · Universal" : download.architecture}`;
  document.querySelector("#download-arrow").textContent = "↓";
  downloadStatus.textContent = `${download.name}${download.size ? ` · ${download.size}` : ""}. See release notes for installation and system requirements.`;
}

async function updateRelease() {
  const result = await fetchLatestRelease();
  if (result.state === "unavailable") {
    document.querySelector("#release-version").textContent = "Release not available";
    downloadStatus.textContent = "A public release could not be found. Check GitHub for updates or build Forma from source.";
    downloadLabel.textContent = "Check releases on GitHub";
    downloadLink.href = RELEASES_URL;
    document.querySelector("#release-notes").href = RELEASES_URL;
    return;
  }
  if (result.state === "error") {
    downloadStatus.textContent = "We couldn’t check the latest release just now. You can still browse downloads and installation notes on GitHub.";
    return;
  }

  const { release } = result;
  document.querySelector("#release-version").textContent = release.version;
  document.querySelector("#release-date").textContent = release.date;
  document.querySelector("#release-badge").textContent = "LATEST RELEASE";
  document.querySelector("#release-notes").href = release.url;
  downloadLink.href = release.url;
  if (release.downloads.length === 0) {
    downloadStatus.textContent = "This release has no packaged Mac download. See its release notes or build Forma from source.";
    return;
  }

  selectDownload(release.downloads[0]);
  if (release.downloads.length > 1) {
    const select = document.querySelector("#build-select");
    release.downloads.forEach((download, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = download.label;
      select.append(option);
    });
    select.addEventListener("change", () => selectDownload(release.downloads[Number(select.value)]));
    document.querySelector("#build-picker").hidden = false;
  }
}

void updateRelease();
