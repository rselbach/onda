// Fills the download card from Onda's signed update feed, choosing the
// package for the visitor's OS. Without this script, or when the feed or OS
// is unusable, the card keeps its "View releases" link.
(() => {
  const FEED_URL = "https://rselbach.github.io/onda/update.json";
  const ASSET_PREFIX = "https://github.com/rselbach/onda/releases/download/";
  const KINDS = ["dmg", "app-zip", "deb", "rpm", "pkg.tar.zst", "tar.gz"];
  const KIND_LABELS = {
    dmg: "DMG",
    "app-zip": "ZIP",
    deb: ".deb",
    rpm: ".rpm",
    "pkg.tar.zst": ".pkg.tar.zst (Arch)",
    "tar.gz": ".tar.gz",
  };
  const GROUPS = [
    { os: "macos", arch: "arm64", label: "macOS (Apple Silicon)" },
    { os: "linux", arch: "x86_64", label: "Linux x86_64" },
    { os: "linux", arch: "arm64", label: "Linux ARM64" },
  ];
  const ARCH_LABELS = { x86_64: "x86_64", arm64: "ARM64" };
  const DOWNLOAD_ICON =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M12 4v12m0 0-5-5m5 5 5-5M5 20h14"/></svg>';

  async function loadRelease() {
    const response = await fetch(FEED_URL);
    if (!response.ok) throw new Error(`feed returned ${response.status}`);
    const { payload } = await response.json();
    const bytes = Uint8Array.from(atob(payload), (c) => c.charCodeAt(0));
    const release = JSON.parse(new TextDecoder().decode(bytes));
    if (typeof release.version !== "string" || !Array.isArray(release.artifacts)) {
      throw new Error("feed payload has no version or artifacts");
    }
    release.artifacts = release.artifacts.filter(
      (a) => typeof a.url === "string" && a.url.startsWith(ASSET_PREFIX),
    );
    return release;
  }

  // returns { os: "macos" | "linux" | "windows" | "other", arch?: "arm64" | "x86_64" }
  async function detectPlatform() {
    const ua = navigator.userAgent;
    const uaData = navigator.userAgentData;
    let os = "other";
    if (uaData && uaData.platform) {
      if (!uaData.mobile) {
        os = { macOS: "macos", Linux: "linux", Windows: "windows" }[uaData.platform] || "other";
      }
    } else if (/iPhone|iPad|iPod|Android|CrOS/.test(ua)) {
      os = "other";
    } else if (/Macintosh|Mac OS X/.test(ua)) {
      // iPadOS asks for desktop sites with a Mac user agent
      os = navigator.maxTouchPoints > 1 ? "other" : "macos";
    } else if (/Linux|X11/.test(ua)) {
      os = "linux";
    } else if (/Windows/.test(ua)) {
      os = "windows";
    }

    let arch;
    if (uaData && uaData.getHighEntropyValues && (os === "macos" || os === "linux")) {
      try {
        const { architecture } = await uaData.getHighEntropyValues(["architecture"]);
        arch = { arm: "arm64", x86: "x86_64" }[architecture];
      } catch {
        // the user agent declined; fall back to the defaults below
      }
    }
    if (!arch && os === "linux") arch = /aarch64|arm64|armv8/i.test(ua) ? "arm64" : "x86_64";
    if (!arch && os === "macos") arch = "arm64"; // Safari always reports Intel
    return { os, arch };
  }

  function pickPrimary(artifacts, { os, arch }) {
    const preferred = { macos: ["dmg", "app-zip"], linux: ["deb", "rpm", "tar.gz"] }[os];
    if (!preferred) return undefined;
    const matching = artifacts.filter((a) => a.os === os && a.arch === arch);
    for (const kind of preferred) {
      const artifact = matching.find((a) => a.kind === kind);
      if (artifact) return artifact;
    }
    return undefined;
  }

  function requirement(primary, platform) {
    if (primary && primary.os === "macos") {
      const major = String(primary.minimum_os || "").replace(/\.0$/, "");
      return major ? `macOS ${major} or later on Apple Silicon` : "Apple Silicon";
    }
    if (primary && primary.os === "linux") {
      return `${KIND_LABELS[primary.kind]} for ${ARCH_LABELS[primary.arch]} · Ubuntu 24.04 or a similarly recent desktop`;
    }
    if (platform.os === "macos") return "Onda needs a Mac with Apple Silicon.";
    return "Onda runs on macOS and Linux.";
  }

  function link(text, href) {
    const a = document.createElement("a");
    a.href = href;
    a.textContent = text;
    return a;
  }

  function megabytes(bytes) {
    return Number.isFinite(bytes) ? `${(bytes / 1e6).toFixed(1)} MB` : "";
  }

  function renderOthers(container, artifacts, primary) {
    const rest = artifacts.filter((a) => a !== primary);
    const heading = document.createElement("span");
    heading.className = "label";
    heading.textContent = primary ? "Other downloads" : "Downloads";
    container.replaceChildren(heading);
    let first = true;
    for (const group of GROUPS) {
      const items = rest
        .filter((a) => a.os === group.os && a.arch === group.arch)
        .sort((a, b) => KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind));
      if (items.length === 0) continue;
      first = false;
      const span = document.createElement("span");
      span.append(`${group.label}: `);
      items.forEach((artifact, i) => {
        if (i > 0) span.append(", ");
        const a = link(KIND_LABELS[artifact.kind] || artifact.kind, artifact.url);
        a.title = megabytes(artifact.size);
        span.append(a);
      });
      container.append(span);
    }
    container.hidden = first;
  }

  async function main() {
    const button = document.getElementById("download");
    const meta = document.getElementById("download-meta");
    const others = document.getElementById("download-others");
    const [release, platform] = await Promise.all([loadRelease(), detectPlatform()]);
    const primary = pickPrimary(release.artifacts, platform);

    if (primary) {
      const name = primary.os === "macos" ? "macOS" : "Linux";
      button.innerHTML = DOWNLOAD_ICON;
      button.append(`Download for ${name}`);
      button.href = primary.url;
      button.title = megabytes(primary.size);
    }

    const version = `Version ${release.version}`;
    const notes = typeof release.notes_url === "string" && release.notes_url.startsWith("https://github.com/rselbach/onda/")
      ? link(version, release.notes_url)
      : version;
    meta.replaceChildren(notes, ` · ${requirement(primary, platform)}`);
    renderOthers(others, release.artifacts, primary);
  }

  main().catch((error) => console.warn("Onda download card:", error));
})();
