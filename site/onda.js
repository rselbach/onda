// Fills the download card from Onda's signed update feed, choosing the
// package for the visitor's OS. Without this script, or when the feed or OS
// is unusable, the card keeps its "View releases" link.
(() => {
  const FEED_URL = "https://rselbach.github.io/onda/update.json";
  const ASSET_PREFIX = "https://github.com/rselbach/onda/releases/download/";
  const NOTES_PREFIX = "https://github.com/rselbach/onda/releases/tag/";
  const PUBLIC_KEY_B64 = "2obyIySbPw8xSuXQwzSw2sCbOixUww08jk95se82unc=";
  const SIGNING_CONTEXT = "onda-update-feed-v1\n";
  const EXPECTED_APP = "com.rselbach.onda";
  const EXPECTED_CHANNEL = "stable";
  const KINDS = ["dmg", "app-zip", "deb", "rpm", "pkg.tar.zst", "tar.gz"];
  const KIND_LABELS = Object.assign(Object.create(null), {
    dmg: "DMG",
    "app-zip": "ZIP",
    deb: ".deb",
    rpm: ".rpm",
    "pkg.tar.zst": ".pkg.tar.zst (Arch)",
    "tar.gz": ".tar.gz",
  });
  const GROUPS = [
    { os: "macos", arch: "arm64", label: "macOS (Apple Silicon)" },
    { os: "linux", arch: "x86_64", label: "Linux x86_64" },
    { os: "linux", arch: "arm64", label: "Linux ARM64" },
  ];
  const ARCH_LABELS = Object.assign(Object.create(null), {
    x86_64: "x86_64",
    arm64: "ARM64",
  });
  const DOWNLOAD_ICON =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M12 4v12m0 0-5-5m5 5 5-5M5 20h14"/></svg>';

  function decodeBase64(value) {
    if (typeof value !== "string" || value.length === 0) {
      throw new Error("expected non-empty base64 string");
    }
    return Uint8Array.from(atob(value), (c) => c.charCodeAt(0));
  }

  async function verifySignature(payloadBytes, signatureB64) {
    if (!globalThis.crypto || !globalThis.crypto.subtle) {
      throw new Error("Web Crypto unavailable for feed verification");
    }
    const keyBytes = decodeBase64(PUBLIC_KEY_B64);
    const sigBytes = decodeBase64(signatureB64);
    if (sigBytes.length !== 64) {
      throw new Error("invalid feed signature length");
    }
    const prefix = new TextEncoder().encode(SIGNING_CONTEXT);
    const signed = new Uint8Array(prefix.length + payloadBytes.length);
    signed.set(prefix, 0);
    signed.set(payloadBytes, prefix.length);
    const key = await globalThis.crypto.subtle.importKey(
      "raw",
      keyBytes,
      { name: "Ed25519" },
      false,
      ["verify"]
    );
    const valid = await globalThis.crypto.subtle.verify(
      "Ed25519",
      key,
      sigBytes,
      signed
    );
    if (!valid) {
      throw new Error("feed signature verification failed");
    }
  }

  async function loadRelease() {
    const response = await fetch(FEED_URL);
    if (!response.ok) throw new Error(`feed returned ${response.status}`);
    const envelope = await response.json();
    if (
      !envelope ||
      typeof envelope !== "object" ||
      typeof envelope.payload !== "string" ||
      typeof envelope.signature !== "string"
    ) {
      throw new Error("feed envelope missing payload or signature");
    }
    const bytes = decodeBase64(envelope.payload);
    await verifySignature(bytes, envelope.signature);
    const release = JSON.parse(new TextDecoder().decode(bytes));
    if (
      !release ||
      typeof release !== "object" ||
      release.schema !== 1 ||
      release.app !== EXPECTED_APP ||
      release.channel !== EXPECTED_CHANNEL ||
      typeof release.version !== "string" ||
      !Array.isArray(release.artifacts)
    ) {
      throw new Error("feed payload failed identity or schema validation");
    }
    release.artifacts = release.artifacts.filter(
      (a) => a && typeof a === "object" && isReleaseAsset(a.url)
    );
    return release;
  }

  // parsed first, so a path like ".../download/../../other/repo" that
  // leaves the release folder is rejected the way the browser resolves it
  function isReleaseAsset(url) {
    try {
      return typeof url === "string" && new URL(url).href.startsWith(ASSET_PREFIX);
    } catch {
      return false;
    }
  }

  function isNotesUrl(url) {
    try {
      if (typeof url !== "string") return false;
      const parsed = new URL(url);
      return (
        parsed.href.startsWith(NOTES_PREFIX) &&
        parsed.href.length > NOTES_PREFIX.length &&
        !parsed.search &&
        !parsed.hash
      );
    } catch {
      return false;
    }
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
    let unsupportedArch = false;
    if (uaData && uaData.getHighEntropyValues && (os === "macos" || os === "linux")) {
      try {
        const { architecture, bitness } = await uaData.getHighEntropyValues([
          "architecture",
          "bitness",
        ]);
        if (bitness === "32") {
          unsupportedArch = true;
        } else if (architecture === "arm") {
          arch = "arm64";
        } else if (architecture === "x86") {
          arch = "x86_64";
        } else if (architecture) {
          unsupportedArch = true;
        }
      } catch {
        // the user agent declined; fall back to the defaults below
      }
    }
    if (!arch && !unsupportedArch && os === "linux") {
      if (/aarch64|arm64|armv8/i.test(ua)) {
        arch = "arm64";
      } else if (/x86_64|amd64|x64|win64/i.test(ua)) {
        arch = "x86_64";
      } else if (
        /i[3-6]86|armv[5-7]|armhf|ppc|powerpc|riscv|s390|mips|sparc|loongarch/i.test(ua)
      ) {
        unsupportedArch = true;
      } else {
        arch = "x86_64";
      }
    }
    if (!arch && !unsupportedArch && os === "macos") arch = "arm64"; // Safari always reports Intel
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
      const kindLabel = Object.hasOwn(KIND_LABELS, primary.kind)
        ? KIND_LABELS[primary.kind]
        : String(primary.kind || "");
      const archLabel = Object.hasOwn(ARCH_LABELS, primary.arch)
        ? ARCH_LABELS[primary.arch]
        : String(primary.arch || "");
      return `${kindLabel} for ${archLabel} · Ubuntu 24.04 or a similarly recent desktop`;
    }
    if (platform.os === "macos" && platform.arch !== "arm64") {
      return "Onda needs a Mac with Apple Silicon.";
    }
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
    const kindRank = (kind) => {
      const index = KINDS.indexOf(kind);
      return index === -1 ? KINDS.length : index;
    };
    let first = true;
    for (const group of GROUPS) {
      const items = rest
        .filter((a) => a.os === group.os && a.arch === group.arch)
        .sort((a, b) => kindRank(a.kind) - kindRank(b.kind));
      if (items.length === 0) continue;
      first = false;
      const span = document.createElement("span");
      span.append(`${group.label}: `);
      items.forEach((artifact, i) => {
        if (i > 0) span.append(", ");
        const label = Object.hasOwn(KIND_LABELS, artifact.kind)
          ? KIND_LABELS[artifact.kind]
          : String(artifact.kind || "");
        const a = link(label, artifact.url);
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
    const notes = isNotesUrl(release.notes_url)
      ? link(version, release.notes_url)
      : version;
    meta.replaceChildren(notes, ` · ${requirement(primary, platform)}`);
    renderOthers(others, release.artifacts, primary);
  }

  main().catch((error) => console.warn("Onda download card:", error));
})();
