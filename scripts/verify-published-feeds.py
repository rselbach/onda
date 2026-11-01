#!/usr/bin/env python3
"""Check that Pages serves the exact signed feeds uploaded by this release."""

import sys
import tarfile
import time
import urllib.error
import urllib.request


def main():
    expected = {}
    with tarfile.open(sys.argv[1]) as archive:
        for member in archive.getmembers():
            name = member.name.removeprefix("./")
            if name in ("update.json", "appcast.xml") and member.isfile():
                with archive.extractfile(member) as source:
                    expected[name] = source.read()
    if set(expected) != {"update.json", "appcast.xml"}:
        raise SystemExit("The Pages artifact must contain both update feeds")

    for attempt in range(12):
        mismatches = []
        for name, wanted in expected.items():
            request = urllib.request.Request(
                f"https://rselbach.github.io/onda/{name}",
                headers={"Cache-Control": "no-cache"},
            )
            try:
                with urllib.request.urlopen(request, timeout=10) as response:
                    actual = response.read(len(wanted) + 1)
                if actual != wanted:
                    mismatches.append(name)
            except (OSError, urllib.error.URLError) as error:
                print(f"{name}: {error}", file=sys.stderr)
                mismatches.append(name)
        if not mismatches:
            print("Both public feeds match the signed release artifact")
            return
        print(f"Attempt {attempt + 1}: stale or unavailable {mismatches}",
              file=sys.stderr)
        if attempt < 11:
            time.sleep(5)
    raise SystemExit("Pages reported success but is not serving the release feeds")


if __name__ == "__main__":
    main()
