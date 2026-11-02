#!/usr/bin/env python3
"""Build native Linux archives and Debian, RPM, and Arch packages without root."""

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NFPM_VERSION = "2.48.0"
NFPM_CHECKSUMS = {
    "x86_64": "6d7f9a28b2f397934403950fcbba83f598b08d02edbe79a0815b3b4897341750",
    "arm64": "a9a75213763783b7669719e505474c748c5ad7dc66591a195619deb50f85c2f4",
}


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def output(*args):
    return run(*args, stdout=subprocess.PIPE).stdout.strip()


def copy(source, destination, mode=0o644):
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(mode)


def nfpm(arch):
    directory = ROOT / "build/tools" / f"nfpm-{NFPM_VERSION}-{arch}"
    binary = directory / "nfpm"
    if binary.is_file():
        return binary
    directory.mkdir(parents=True, exist_ok=True)
    name = f"nfpm_{NFPM_VERSION}_Linux_{arch}.tar.gz"
    url = f"https://github.com/goreleaser/nfpm/releases/download/v{NFPM_VERSION}/{name}"
    with urllib.request.urlopen(url, timeout=120) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != NFPM_CHECKSUMS[arch]:
        raise ValueError("nFPM download checksum mismatch")
    archive = directory / name
    archive.write_bytes(data)
    with tarfile.open(archive) as tar:
        tar.extract("nfpm", directory, filter="data")
    binary.chmod(0o755)
    archive.unlink()
    return binary


def third_party_notices(stage, target):
    metadata = json.loads(
        output(
            "cargo",
            "metadata",
            "--locked",
            "--format-version=1",
            "--filter-platform",
            target,
        )
    )
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [p["id"] for p in packages.values() if p["name"] == "onda-linux"]
    visited = set()
    notices = []
    while pending:
        package_id = pending.pop()
        if package_id in visited:
            continue
        visited.add(package_id)
        pending.extend(nodes[package_id]["dependencies"])
        package = packages[package_id]
        if package["source"] is None:
            continue
        name = f"{package['name']}-{package['version']}"
        notices.append(f"{name}: {package['license'] or 'see license files'}")
        source = Path(package["manifest_path"]).parent
        for path in source.rglob("*"):
            if path.is_file() and re.match(
                r"^(LICENSE|LICENCE|COPYING|NOTICE)([.-]|$)", path.name, re.IGNORECASE
            ):
                copy(
                    path,
                    stage / "share/doc/onda/licenses" / name / path.relative_to(source),
                )
    (stage / "share/doc/onda/THIRD-PARTY.txt").write_text(
        "\n".join(sorted(notices)) + "\n"
    )


def dependencies(binary, library):
    # Record the build host's library versions. A build on a newer distribution
    # must not advertise the lower requirements of the official Ubuntu builds.
    versions = {
        name: output("pkg-config", "--modversion", name)
        for name in ("gtk4", "libadwaita-1", "glib-2.0", "alsa")
    }
    symbols = "\n".join(
        output("readelf", "--version-info", str(p)) for p in (binary, library)
    )
    glibc = max(
        re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)", symbols),
        key=lambda v: tuple(map(int, v.split("."))),
    )
    cxx = output(os.environ.get("CXX", "c++"), "-dumpfullversion", "-dumpversion")
    common = [
        "wl-clipboard",
        "wtype",
        "xclip",
        "xdotool",
        "xdg-utils",
        "xdg-desktop-portal",
    ]
    deb = common + [
        f"libc6 (>= {glibc})",
        f"libstdc++6 (>= {cxx})",
        "libgcc-s1",
        f"libgtk-4-1 (>= {versions['gtk4']})",
        f"libadwaita-1-0 (>= {versions['libadwaita-1']})",
        f"libglib2.0-0t64 (>= {versions['glib-2.0']})",
        f"libasound2t64 (>= {versions['alsa']})",
        "libsecret-tools",
    ]
    rpm = common + [
        f"glibc >= {glibc}",
        f"gtk4 >= {versions['gtk4']}",
        f"libadwaita >= {versions['libadwaita-1']}",
        f"glib2 >= {versions['glib-2.0']}",
        f"alsa-lib >= {versions['alsa']}",
        "libsecret",
    ]
    archlinux = common + [
        f"glibc>={glibc}",
        f"gcc-libs>={cxx}",
        f"gtk4>={versions['gtk4']}",
        f"libadwaita>={versions['libadwaita-1']}",
        f"glib2>={versions['glib-2.0']}",
        f"alsa-lib>={versions['alsa']}",
        "libsecret",
        "libx11",
        "libxtst",
        "libxkbcommon",
        "wayland",
    ]
    # nFPM does not automatically generate ELF dependency requirements.
    for path in (binary, library):
        for soname in re.findall(
            r"\(NEEDED\).*?\[(.*?)\]", output("readelf", "-d", str(path))
        ):
            if soname != "libgtk4-layer-shell.so.0":
                rpm.append(f"{soname}()(64bit)")
    for symbol in sorted(set(re.findall(r"\b(?:GLIBCXX|CXXABI)_[\d.]+", symbols))):
        rpm.append(f"libstdc++.so.6({symbol})(64bit)")
    return deb, sorted(set(rpm)), archlinux, versions, glibc


def package(version, arch, stage, destination, target):
    binary = stage / "bin/onda"
    library = stage / "lib/onda/libgtk4-layer-shell.so.0"
    copy(ROOT / "build/linux-target/release/onda", binary, 0o755)
    libdir = Path(output("pkg-config", "--variable=libdir", "gtk4-layer-shell-0"))
    prefix = Path(output("pkg-config", "--variable=prefix", "gtk4-layer-shell-0"))
    copy(libdir / "libgtk4-layer-shell.so.0", library, 0o755)
    run("strip", "--strip-unneeded", str(binary), str(library))
    if output(str(binary), "--version") != f"Onda {version}":
        raise ValueError("Packaged binary has the wrong version")
    copy(
        ROOT / "shell/linux/com.rselbach.onda.desktop",
        stage / "share/applications/com.rselbach.onda.desktop",
    )
    copy(
        ROOT / "assets/AppIcon.appiconset/icon_256x256.png",
        stage / "share/icons/hicolor/256x256/apps/com.rselbach.onda.png",
    )
    copy(ROOT / "LICENSE", stage / "share/doc/onda/copyright")
    copy(
        ROOT / "shell/linux/licenses/hex.txt", stage / "share/doc/onda/licenses/hex.txt"
    )
    copy(
        prefix / "share/licenses/gtk4-layer-shell/LICENSE",
        stage / "share/doc/onda/licenses/gtk4-layer-shell.txt",
    )
    copy(ROOT / "shell/linux/packaging/README.txt", stage / "share/doc/onda/README.txt")
    third_party_notices(stage, target)
    deb, rpm, archlinux, versions, glibc = dependencies(binary, library)
    requirements = {
        "version": version,
        "architecture": arch,
        "glibc": glibc,
        **versions,
    }
    (stage / "share/doc/onda/build.json").write_text(
        json.dumps(requirements, indent=2) + "\n"
    )
    config = {
        "name": "onda",
        "arch": "amd64" if arch == "x86_64" else "arm64",
        "platform": "linux",
        "version": version,
        "release": "1",
        "maintainer": "Roberto Selbach <rselbach@rselbach.com>",
        "description": "Local voice dictation with optional cloud transcript refinement.",
        "homepage": "https://github.com/rselbach/onda",
        "license": "MIT",
        "section": "sound",
        "contents": [
            {"src": str(p), "dst": "/usr/" + str(p.relative_to(stage))}
            for p in sorted(stage.rglob("*"))
            if p.is_file()
        ],
        "overrides": {
            "deb": {"depends": deb, "recommends": ["gnome-keyring"]},
            "rpm": {"depends": rpm, "recommends": ["gnome-keyring"]},
            "archlinux": {"depends": archlinux, "recommends": ["gnome-keyring"]},
        },
        "archlinux": {"packager": "Roberto Selbach <rselbach@rselbach.com>"},
    }
    config_path = stage.parent / "nfpm.json"
    formats = {"deb": "deb", "rpm": "rpm", "archlinux": "pkg.tar.zst"}
    for packager, kind in formats.items():
        marker = stage.parent / f"{packager}-package-kind"
        marker.write_text(kind + "\n")
        config["overrides"][packager]["contents"] = config["contents"] + [
            {"src": str(marker), "dst": "/usr/lib/onda/package-kind"}
        ]
    config["overrides"]["archlinux"]["contents"].append(
        {"src": str(ROOT / "LICENSE"), "dst": "/usr/share/licenses/onda/LICENSE"}
    )
    tool = nfpm(arch)
    artifacts = []
    for packager, kind in formats.items():
        package_config = config
        if packager == "archlinux":
            # Preserve prereleases: nFPM 2.48 drops them from Arch's .PKGINFO
            # when parsing semver. Arch pkgver cannot contain hyphens.
            package_config = config | {
                "version_schema": "none",
                "version": version.replace("-", "", 1).replace("-", "_"),
            }
        config_path.write_text(json.dumps(package_config, indent=2))
        artifact = destination / f"{stage.name}.{kind}"
        run(
            str(tool),
            "package",
            "--config",
            str(config_path),
            "--packager",
            packager,
            "--target",
            str(artifact),
        )
        artifacts.append(artifact)
    copy(ROOT / "shell/linux/packaging/install.sh", stage / "install.sh", 0o755)
    (stage / "lib/onda/package-kind").write_text("tar.gz\n")
    copy(ROOT / "shell/linux/packaging/README.txt", stage / "README.txt")
    archive = destination / f"{stage.name}.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(stage, arcname=stage.name)
    artifacts.append(archive)
    sums = destination / f"{stage.name}.sha256"
    checksums = []
    for artifact in artifacts:
        with artifact.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        checksums.append(f"{digest}  {artifact.name}\n")
    sums.write_text("".join(checksums))
    print(f"Packages and checksums: {destination}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="release version, e.g. 3.2.0 or 3.2.0-rc.1")
    args = parser.parse_args()
    args.version = args.version.removeprefix("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", args.version):
        parser.error("version must be X.Y.Z with an optional prerelease suffix")
    machine = platform.machine()
    if platform.system() != "Linux" or machine not in ("x86_64", "aarch64"):
        parser.error("run natively on x86_64 or aarch64 Linux")
    arch = "arm64" if machine == "aarch64" else machine
    target = f"{machine}-unknown-linux-gnu"
    os.chdir(ROOT)
    env = os.environ.copy()
    env["ONDA_VERSION"] = args.version
    env["CARGO_TARGET_DIR"] = str(ROOT / "build/linux-target")
    # Keep release CMake caches separate from host-tuned development builds.
    # x86_64 uses AVX2/FMA/F16C; ARM64 uses the ARMv8.0-A baseline.
    env["TRANSCRIBE_CMAKE_ARGS"] = " ".join(
        [
            "-DGGML_NATIVE=OFF",
            "-DTRANSCRIBE_X86_CONSERVATIVE=ON",
            "-DGGML_CPU_ARM_ARCH=armv8-a",
            "-DTRANSCRIBE_USE_OPENMP=OFF",
            *(
                ["-DGGML_AVX=ON", "-DGGML_AVX2=ON", "-DGGML_FMA=ON", "-DGGML_F16C=ON"]
                if arch == "x86_64"
                else []
            ),
        ]
    )
    # Reject overrides which would bypass or change the documented CPU floor.
    for key in (
        "TRANSCRIBE_DIR",
        "CMAKE_ARGS",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CFLAGS",
        "CXXFLAGS",
        "CARGO_BUILD_TARGET",
    ):
        if env.get(key):
            parser.error(f"unset {key} when building release packages")
    run("cargo", "build", "--locked", "--release", "-p", "onda-linux", env=env)
    destination = ROOT / "build/linux"
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="package-", dir=destination) as temporary:
        stage = Path(temporary) / f"Onda-{args.version}-linux-{arch}"
        package(args.version, arch, stage, destination, target)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        sys.exit(1)
