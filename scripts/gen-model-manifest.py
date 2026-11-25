#!/usr/bin/env python3
"""Regenerates crates/onda-asr-coreml/src/manifest.rs from Hugging Face.

Lists every file the CoreML backend needs with its size and SHA-256 (DL-5,
DL-6). LFS files carry their hash in the tree listing; the small git files are
downloaded and hashed here, so the app can verify a whole folder before
CoreML reads it.

Each model stays on the revision already pinned in the manifest, so a
regeneration never moves a model silently; the script only reports when
upstream has moved. Pass --repin to move every model to its repository's
current commit, then review the diff.
"""

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "crates/onda-asr-coreml/src/manifest.rs"
REVISION_RE = re.compile(r"^[0-9a-f]{40}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
SEGMENT_RE = re.compile(r"^[A-Za-z0-9._-]+$")

# model id -> (repo, required top-level entries); mirrors FluidAudio's
# ModelNames required sets for the variants Jabber loads
MODELS = [
    ("ParakeetTdtV2", "FluidInference/parakeet-tdt-0.6b-v2-coreml", [
        "Preprocessor.mlmodelc", "Encoder.mlmodelc", "Decoder.mlmodelc",
        "JointDecision.mlmodelc", "parakeet_vocab.json", "config.json",
    ]),
    ("ParakeetUltra", "FluidInference/parakeet-ultra-coreml", [
        "Preprocessor.mlmodelc", "Encoder.mlmodelc", "Decoder.mlmodelc",
        "JointDecisionv3.mlmodelc", "parakeet_vocab.json", "config.json",
    ]),
    ("ParakeetJapanese", "FluidInference/parakeet-0.6b-ja-coreml", [
        "Preprocessor.mlmodelc", "Encoder.mlmodelc", "Decoderv2.mlmodelc",
        "Jointerv2.mlmodelc", "vocab.json", "config.json", "metadata.json",
    ]),
    ("ParakeetUnified", "FluidInference/parakeet-unified-en-0.6b-coreml", [
        "parakeet_unified_encoder_streaming_70_7_7_int8.mlmodelc",
        "parakeet_unified_decoder.mlmodelc",
        "parakeet_unified_joint_decision_single_step.mlmodelc",
        "vocab.json", "metadata.json", "config.json",
    ]),
]


def fetch_json(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.load(response)


def fetch_sha256(url, size):
    digest = hashlib.sha256()
    read = 0
    with urllib.request.urlopen(url, timeout=60) as response:
        while chunk := response.read(1 << 20):
            digest.update(chunk)
            read += len(chunk)
    if read != size:
        sys.exit(f"{url}: got {read} bytes, the tree lists {size}")
    return digest.hexdigest()


def pinned_revisions():
    """Repository -> revision pinned in the current manifest."""
    if not os.path.exists(OUTPUT):
        return {}
    with open(OUTPUT) as handle:
        text = handle.read()
    pattern = r'base_url: "https://huggingface\.co/([^"]+)/resolve/([0-9a-f]+)"'
    return dict(re.findall(pattern, text))


def validate_revision(repo, revision):
    if not isinstance(revision, str) or not REVISION_RE.fullmatch(revision):
        sys.exit(f"{repo}: invalid revision {revision!r}")
    return revision


def validate_sha256(repo, path, sha):
    if not isinstance(sha, str) or not SHA256_RE.fullmatch(sha):
        sys.exit(f"{repo}/{path}: invalid sha256 {sha!r}")
    return sha


def validate_size(repo, path, size):
    if not isinstance(size, int) or isinstance(size, bool) or size < 0:
        sys.exit(f"{repo}/{path}: invalid size {size!r}")
    return size


def validate_path(repo, path):
    if not isinstance(path, str) or not path or path.startswith("/") or path.endswith("/"):
        sys.exit(f"{repo}: invalid path {path!r}")
    for segment in path.split("/"):
        if segment in ("", ".", "..") or not SEGMENT_RE.fullmatch(segment):
            sys.exit(f"{repo}: invalid path segment {segment!r} in {path!r}")
    return path


def manifest_for(variant, repo, required, pinned):
    upstream = validate_revision(
        repo, fetch_json(f"https://huggingface.co/api/models/{repo}")["sha"]
    )
    revision = validate_revision(repo, pinned or upstream)
    if revision != upstream:
        print(f"note: {repo} moved upstream to {upstream[:12]}; "
              f"keeping {revision[:12]} (pass --repin to move)", file=sys.stderr)
    tree = fetch_json(
        f"https://huggingface.co/api/models/{repo}/tree/{revision}?recursive=1")
    files = []
    for item in tree:
        if item["type"] != "file":
            continue
        path = validate_path(repo, item["path"])
        if path.split("/")[0] not in required:
            continue
        lfs = item.get("lfs")
        if lfs:
            size = validate_size(repo, path, lfs["size"])
            sha = validate_sha256(repo, path, lfs["oid"])
            files.append((path, size, sha))
            continue
        size = validate_size(repo, path, item["size"])
        encoded_path = urllib.parse.quote(path, safe="/")
        url = f"https://huggingface.co/{repo}/resolve/{revision}/{encoded_path}"
        sha = validate_sha256(repo, path, fetch_sha256(url, size))
        files.append((path, size, sha))
    found = {path.split("/")[0] for path, _, _ in files}
    missing = sorted(set(required) - found)
    if missing:
        sys.exit(f"{repo}@{revision} is missing {missing}")
    files.sort()
    return variant, repo, revision, files


def render(manifests):
    out = [
        "//! Download manifests for the CoreML conversions (DL-5, DL-6, MOD-1).",
        "//!",
        "//! Generated by `scripts/gen-model-manifest.py`; do not edit by hand.",
        "",
        "use onda_asr::{ManifestFile, ModelManifest};",
        "use onda_domain::models::ModelId;",
        "",
        "/// The files the CoreML backend needs for `model`; `None` for engines",
        "/// that download nothing.",
        "pub fn manifest(model: ModelId) -> Option<&'static ModelManifest> {",
        "    match model {",
    ]
    for variant, _, _, _ in manifests:
        out.append(f"        ModelId::{variant} => Some(&{variant.upper()}),")
    out += ["        ModelId::SystemSpeech => None,", "    }", "}", ""]
    for variant, repo, revision, files in manifests:
        total = sum(size for _, size, _ in files)
        out.append(f"// {repo}: {len(files)} files, {total} bytes")
        out.append(f"static {variant.upper()}: ModelManifest = ModelManifest {{")
        out.append(f'    base_url: "https://huggingface.co/{repo}/resolve/{revision}",')
        out.append("    files: &[")
        for path, size, sha in files:
            out.append("        ManifestFile {")
            out.append(f'            path: "{path}",')
            out.append(f"            size: {size},")
            out.append(f'            sha256: Some("{sha}"),')
            out.append("        },")
        out.append("    ],")
        out.append("};")
        out.append("")
    return "\n".join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repin", action="store_true",
                        help="move every model to its current upstream commit")
    args = parser.parse_args()
    pins = {} if args.repin else pinned_revisions()
    manifests = [manifest_for(variant, repo, required, pins.get(repo))
                 for variant, repo, required in MODELS]
    rendered = render(manifests)
    tmp_path = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            dir=OUTPUT.parent,
            prefix=".manifest.",
            suffix=".tmp",
            delete=False,
        ) as handle:
            tmp_path = handle.name
            handle.write(rendered)
        os.chmod(tmp_path, 0o644)
        os.replace(tmp_path, OUTPUT)
        tmp_path = None
    finally:
        if tmp_path is not None:
            try:
                os.unlink(tmp_path)
            except OSError:
                pass
    for variant, repo, revision, files in manifests:
        total = sum(size for _, size, _ in files)
        print(f"{variant:16} {total / 1e6:7.1f} MB  {repo}@{revision[:12]}")


if __name__ == "__main__":
    main()
