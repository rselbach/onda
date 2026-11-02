#!/usr/bin/env python3
"""Launch a packaged Onda in an isolated X11/D-Bus session and verify control."""

import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="onda-launch-") as temporary:
        scratch = Path(temporary)
        runtime = scratch / "runtime"
        runtime.mkdir(mode=0o700)
        env = os.environ.copy()
        env.update(
            {
                "ONDA_DATA_DIR": str(scratch / "onda"),
                "XDG_RUNTIME_DIR": str(runtime),
                "XDG_CONFIG_HOME": str(scratch / "config"),
                "XDG_DATA_HOME": str(scratch / "data"),
                "GDK_BACKEND": "x11",
                "GSK_RENDERER": "cairo",
                "GTK_A11Y": "none",
            }
        )
        env.pop("WAYLAND_DISPLAY", None)
        with tempfile.TemporaryFile(mode="w+") as log:
            app = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
            try:
                for _ in range(60):
                    if app.poll() is not None:
                        raise RuntimeError(
                            f"Onda exited during startup: {app.returncode}"
                        )
                    status = subprocess.run(
                        [binary, "--status"],
                        check=False,
                        env=env,
                        capture_output=True,
                        text=True,
                        timeout=5,
                    )
                    if status.returncode == 0 and status.stdout.strip():
                        break
                    time.sleep(0.25)
                else:
                    raise RuntimeError("Onda did not accept status commands")
                subprocess.run([binary, "--quit"], env=env, check=True, timeout=10)
                assert app.wait(timeout=10) == 0
                print("Packaged app started, answered status, and quit cleanly.")
            finally:
                if app.poll() is None:
                    app.kill()
                    app.wait()
                log.seek(0)
                print(log.read(), end="")


if __name__ == "__main__":
    main()
