#!/usr/bin/env python3
"""Check real release package payloads, relocation, and per-user upgrades."""

import hashlib
import io
import os
import platform
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, **kwargs).stdout


def verify(prefix, version, kind="tar.gz"):
    binary = prefix / "bin/onda"
    assert (prefix / "share/doc/onda/package-kind").read_text().strip() == kind
    env = os.environ.copy()
    env.pop("LD_LIBRARY_PATH", None)
    env.pop("LD_PRELOAD", None)
    assert run(str(binary), "--version", env=env).decode().strip() == f"Onda {version}"
    linked = run("ldd", str(binary), env=env).decode()
    assert "not found" not in linked, linked
    layer = next(
        line for line in linked.splitlines() if "libgtk4-layer-shell.so.0" in line
    )
    assert str(prefix) in layer, layer
    assert "$ORIGIN/../lib/onda" in run("readelf", "-d", str(binary)).decode()
    for path in (
        "share/applications/com.rselbach.onda.desktop",
        "share/icons/hicolor/256x256/apps/com.rselbach.onda.png",
        "share/doc/onda/copyright",
        "share/doc/onda/THIRD-PARTY.txt",
        "share/doc/onda/licenses/gtk4-layer-shell.txt",
        "share/doc/onda/build.json",
    ):
        assert (prefix / path).is_file(), path
    run(
        "desktop-file-validate",
        str(prefix / "share/applications/com.rselbach.onda.desktop"),
    )


def main():
    version = sys.argv[1].removeprefix("v")
    arch = "arm64" if platform.machine() == "aarch64" else "x86_64"
    name = f"Onda-{version}-linux-{arch}"
    directory = ROOT / "build/linux"
    run("sha256sum", "--check", name + ".sha256", cwd=directory)
    with tempfile.TemporaryDirectory(prefix="onda-package-test-") as temporary:
        work = Path(temporary)
        with tarfile.open(directory / f"{name}.tar.gz") as tar:
            tar.extractall(work, filter="data")
        archive = work / name
        verify(archive, version)
        run(
            "xvfb-run",
            "-a",
            "dbus-run-session",
            "--config-file=" + str(ROOT / "shell/linux/tests/dbus-session.conf"),
            "--",
            sys.executable,
            str(ROOT / "scripts/test-linux-package-launch.py"),
            str(archive / "bin/onda"),
        )
        installed = work / "install with spaces"
        for _ in range(2):
            run(str(archive / "install.sh"), str(installed))
            verify(installed, version)

        deb = directory / f"{name}.deb"
        members = run("ar", "t", str(deb)).decode().splitlines()
        data = next(member for member in members if member.startswith("data.tar"))
        with tarfile.open(fileobj=io.BytesIO(run("ar", "p", str(deb), data))) as tar:
            tar.extractall(work / "deb", filter="data")
        control = next(member for member in members if member.startswith("control.tar"))
        with tarfile.open(fileobj=io.BytesIO(run("ar", "p", str(deb), control))) as tar:
            member = next(m for m in tar if m.name.removeprefix("./") == "control")
            metadata = tar.extractfile(member).read().decode()
            assert "Package: onda\n" in metadata, metadata
            assert (
                f"Architecture: {'arm64' if arch == 'arm64' else 'amd64'}\n" in metadata
            ), metadata
            assert "Depends:" in metadata, metadata
        verify(work / "deb/usr", version, "deb")

        rpm = directory / f"{name}.rpm"
        rpm_arch = "aarch64" if arch == "arm64" else "x86_64"
        assert (
            run("rpm", "-qp", "--queryformat", "%{NAME} %{ARCH}", str(rpm)).decode()
            == f"onda {rpm_arch}"
        )
        assert b"libgtk-4.so.1()(64bit)" in run("rpm", "-qp", "--requires", str(rpm))
        rpm_root = work / "rpm"
        rpm_root.mkdir()
        payload = run("rpm2cpio", str(rpm))
        run(
            "cpio",
            "--extract",
            "--make-directories",
            "--no-absolute-filenames",
            input=payload,
            cwd=rpm_root,
        )
        verify(rpm_root / "usr", version, "rpm")
        binaries = [
            archive / "bin/onda",
            work / "deb/usr/bin/onda",
            rpm_root / "usr/bin/onda",
        ]
        assert (
            len({hashlib.sha256(path.read_bytes()).hexdigest() for path in binaries})
            == 1
        )
    print("All package payloads, private library loading, and archive upgrades passed.")


if __name__ == "__main__":
    main()
