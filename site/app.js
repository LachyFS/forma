// Progressive enhancements. All primary links also work without JavaScript.
import { fetchLatestRelease, RELEASES_URL } from "./releases.mjs";

const previews = {
  workspace: {
    src: "./assets/workspace.webp",
    alt: "Forma's native workspace with a teal torus, metallic sphere, and rounded cube on a studio plinth, surrounded by the scene outliner and material inspector.",
    caption: "From a first primitive to a finished perspective.",
  },
  lighting: {
    src: "./assets/lighting.webp",
    alt: "The same Forma scene lit by the Courtyard HDR environment, with controls for lighting rotation, strength, background blur, and contact shading.",
    caption: "Change the atmosphere. See your idea in a new light.",
  },
  commands: {
    src: "./assets/commands.webp",
    alt: "Forma's command palette open over the studio scene, searching for the Wireframe viewport command.",
    caption: "A command away from whatever comes next.",
  },
};

const previewButtons = [...document.querySelectorAll("[data-preview]")];
const previewImage = document.querySelector("#workspace-image");
const previewDescription = document.querySelector("#preview-description");
let previewRequest = 0;

previewButtons.forEach((button) => {
  button.addEventListener("click", async () => {
    const preview = previews[button.dataset.preview];
    const request = ++previewRequest;
    const nextImage = new Image();
    nextImage.src = preview.src;
    try {
      await nextImage.decode();
    } catch {
      // Leave the working image and selected button intact if an asset fails.
      return;
    }
    if (request !== previewRequest) return;
    previewImage.src = preview.src;
    previewImage.alt = preview.alt;
    previewImage.closest("figure").querySelector("figcaption").textContent = preview.alt;
    previewDescription.textContent = preview.caption;
    previewButtons.forEach((item) => item.setAttribute("aria-pressed", String(item === button)));
  });
});
document.querySelector(".preview-switcher").hidden = false;

const downloadLink = document.querySelector("#release-download");
const downloadLabel = document.querySelector("#download-label");
const downloadStatus = document.querySelector("#download-status");

function selectDownload(download) {
  downloadLink.href = download.url;
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
