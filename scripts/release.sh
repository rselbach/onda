#!/usr/bin/env bash
#
# Builds a release Onda.app with scripts/bundle-app.sh, notarizes and
# staples the app, then packages that same stapled bundle twice (REL-3,
# UPD-5): as the ZIP the in-app updater installs and as a compressed,
# signed, notarized, and stapled DMG with an /Applications symlink. Both are
# verified the way a user's Mac and the updater check them. Writes
# build/Onda-<version>-macos-arm64.zip and build/Onda-<version>.dmg.
#
# Version and build number come from --version/--build, else VERSION/BUILD in
# the environment, else shell/macos/Info.plist. Notarization authenticates
# with APPLE_ID, APPLE_TEAM_ID, and APPLE_APP_PASSWORD when all are set, else
# with the notarytool keychain profile NOTARY_PROFILE (default "onda-notary"):
#
#   xcrun notarytool store-credentials onda-notary \
#     --apple-id <apple-id> --team-id <team-id> --password <app-password>
#
# Usage: scripts/release.sh [--skip-notarize] [--version X.Y.Z] [--build N]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
readonly BUILD_DIR="${ROOT}/build"
readonly APP="${BUILD_DIR}/Onda.app"
readonly SOURCE_PLIST="${ROOT}/shell/macos/Info.plist"
readonly UPDATE_REQUIREMENT="${ROOT}/crates/onda-update/update-requirement.txt"
readonly VOLUME_NAME="Onda"
STAGING=""
MOUNT_POINT=""

usage() {
  echo "usage: $0 [--skip-notarize] [--version X.Y.Z] [--build N]" >&2
  exit 2
}

die() {
  echo "error: $*" >&2
  exit 1
}

# keeps the failing status: bash 3.2 (macOS's /bin/bash) otherwise exits with
# the trap's last status, so a failed build would report success
cleanup() {
  local status=$?
  if [[ -n "${MOUNT_POINT}" ]]; then
    hdiutil detach -quiet "${MOUNT_POINT}" || true
  fi
  if [[ -n "${STAGING}" ]]; then
    rm -rf "${STAGING}"
  fi
  exit "${status}"
}

plist_value() {
  local key="$1"
  /usr/libexec/PlistBuddy -c "Print :${key}" "${SOURCE_PLIST}"
}

require_tools() {
  local tools=(cargo swift xcodebuild codesign hdiutil spctl xcrun ditto)
  local tool
  for tool in "${tools[@]}"; do
    if ! command -v "${tool}" >/dev/null; then
      die "required command '${tool}' not found"
    fi
  done
}

uses_apple_id() {
  [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" \
    && -n "${APPLE_APP_PASSWORD:-}" ]]
}

check_notary_credentials() {
  local partial="${APPLE_ID:-}${APPLE_TEAM_ID:-}${APPLE_APP_PASSWORD:-}"
  if [[ -n "${partial}" ]] && ! uses_apple_id; then
    die "set all of APPLE_ID, APPLE_TEAM_ID, and APPLE_APP_PASSWORD"
  fi
}

# the app's signing authority, or "-" for an ad hoc signature
app_identity() {
  local info
  info="$(codesign --display --verbose=2 "${APP}" 2>&1)"
  local authority
  authority="$(sed -n 's/^Authority=//p' <<<"${info}" | head -n 1)"
  if [[ -z "${authority}" ]]; then
    echo "-"
    return
  fi
  echo "${authority}"
}

create_dmg() {
  local dmg="$1"
  local identity="$2"
  local root="${STAGING}/dmg"
  mkdir -p "${root}"
  ditto "${APP}" "${root}/Onda.app"
  ln -s /Applications "${root}/Applications"
  rm -f "${dmg}"
  # lzma-compressed read-only image; supported on every macOS Onda runs on
  hdiutil create -quiet -volname "${VOLUME_NAME}" -srcfolder "${root}" \
    -fs APFS -format ULMO "${dmg}"
  if [[ "${identity}" == "-" ]]; then
    echo "app is signed ad hoc; leaving the DMG unsigned" >&2
    return
  fi
  codesign --force --timestamp --sign "${identity}" "${dmg}"
  codesign --verify --strict --verbose=2 "${dmg}"
}

# submits $1 (a ZIP or DMG) and staples the ticket to $2
notarize() {
  local submission="$1"
  local target="$2"
  local auth=(--keychain-profile "${NOTARY_PROFILE:-onda-notary}")
  if uses_apple_id; then
    auth=(--apple-id "${APPLE_ID}" --team-id "${APPLE_TEAM_ID}"
      --password "${APPLE_APP_PASSWORD}")
  fi

  # the submission's status decides, not notarytool's exit code
  local result="${STAGING}/notary.json"
  if ! xcrun notarytool submit "${submission}" "${auth[@]}" --wait \
    --output-format json >"${result}"; then
    echo "notarytool submit failed" >&2
  fi
  local status=""
  status="$(plutil -extract status raw -o - "${result}")" || true
  if [[ "${status}" != "Accepted" ]]; then
    local id=""
    id="$(plutil -extract id raw -o - "${result}")" || true
    if [[ -z "${id}" ]]; then
      cat "${result}" >&2
      die "notarization did not complete"
    fi
    echo "notarization ${status} (submission ${id}); log:" >&2
    xcrun notarytool log "${id}" "${auth[@]}" >&2 || true
    exit 1
  fi
  xcrun stapler staple "${target}"
  xcrun stapler validate "${target}"
}

# the notary service takes a ZIP of the app; the ticket is stapled to the
# app itself so the update ZIP carries it
notarize_app() {
  local submission="${STAGING}/Onda-notarize.zip"
  ditto -c -k --keepParent "${APP}" "${submission}"
  notarize "${submission}" "${APP}"
  rm -f "${submission}"
}

create_zip() {
  local zip="$1"
  rm -f "${zip}"
  ditto -c -k --sequesterRsrc --keepParent "${APP}" "${zip}"
}

# unpacks the ZIP the way the updater does and checks what it checks
verify_zip() {
  local zip="$1"
  local version="$2"
  local build="$3"
  local notarized="$4"
  local unpacked="${STAGING}/zip"
  mkdir -p "${unpacked}"
  ditto -x -k "${zip}" "${unpacked}"
  local app="${unpacked}/Onda.app"
  if [[ ! -d "${app}" ]]; then
    die "ZIP has no Onda.app"
  fi
  local plist="${app}/Contents/Info.plist"
  local got_version
  local got_build
  got_version="$(plutil -extract CFBundleShortVersionString raw -o - "${plist}")"
  got_build="$(plutil -extract CFBundleVersion raw -o - "${plist}")"
  if [[ "${got_version}" != "${version}" || "${got_build}" != "${build}" ]]; then
    die "ZIP holds ${got_version} (${got_build}), not ${version} (${build})"
  fi
  codesign --verify --deep --strict --verbose=2 "${app}"
  if [[ "${notarized}" == "true" ]]; then
    local requirement
    requirement="$(<"${UPDATE_REQUIREMENT}")"
    codesign --verify --deep --strict -R="${requirement}" "${app}"
    xcrun stapler validate "${app}"
    spctl --assess --type execute --verbose=2 "${app}"
  fi
  rm -rf "${unpacked}"
}

# mounts the DMG read-only and checks what a user would get from it;
# Gatekeeper rejections are fatal only for a notarized build
verify_dmg() {
  local dmg="$1"
  local notarized="$2"
  hdiutil verify -quiet "${dmg}"
  if [[ "${notarized}" == "true" ]]; then
    xcrun stapler validate "${dmg}"
    spctl --assess --type open --context context:primary-signature \
      --verbose=2 "${dmg}"
  fi

  MOUNT_POINT="${STAGING}/mount"
  mkdir -p "${MOUNT_POINT}"
  hdiutil attach -quiet -readonly -nobrowse -noautoopen \
    -mountpoint "${MOUNT_POINT}" "${dmg}"
  if [[ ! -d "${MOUNT_POINT}/Onda.app" ]]; then
    die "DMG has no Onda.app"
  fi
  local link
  link="$(readlink "${MOUNT_POINT}/Applications")" || true
  if [[ "${link}" != "/Applications" ]]; then
    die "DMG has no /Applications symlink"
  fi
  codesign --verify --deep --strict --verbose=2 "${MOUNT_POINT}/Onda.app"
  if [[ "${notarized}" == "true" ]]; then
    spctl --assess --type execute --verbose=2 "${MOUNT_POINT}/Onda.app"
  elif ! spctl --assess --type execute --verbose=2 \
    "${MOUNT_POINT}/Onda.app"; then
    echo "Gatekeeper rejects the app (expected without notarization)" >&2
  fi
  hdiutil detach -quiet "${MOUNT_POINT}"
  MOUNT_POINT=""
}

main() {
  local notarize_dmg="true"
  local version="${VERSION:-}"
  local build="${BUILD:-}"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --skip-notarize) notarize_dmg="false" ;;
      --version)
        [[ $# -ge 2 ]] || usage
        version="$2"
        shift
        ;;
      --build)
        [[ $# -ge 2 ]] || usage
        build="$2"
        shift
        ;;
      *) usage ;;
    esac
    shift
  done

  if [[ -z "${version}" ]]; then
    version="$(plist_value CFBundleShortVersionString)"
  fi
  if [[ -z "${build}" ]]; then
    build="$(plist_value CFBundleVersion)"
  fi
  if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
    die "version '${version}' is not X.Y.Z"
  fi
  # the update feed (and the legacy Sparkle appcast) order releases by
  # CFBundleVersion
  if [[ ! "${build}" =~ ^[0-9]+$ ]]; then
    die "build '${build}' is not a number"
  fi

  require_tools
  if [[ "${notarize_dmg}" == "true" ]]; then
    check_notary_credentials
  fi

  STAGING="$(mktemp -d)"
  trap cleanup EXIT

  VERSION="${version}" BUILD="${build}" \
    "${ROOT}/scripts/bundle-app.sh" release

  local identity
  identity="$(app_identity)"
  if [[ "${notarize_dmg}" == "true" \
    && "${identity}" != "Developer ID Application"* ]]; then
    die "notarization needs a Developer ID signature; app has '${identity}'"
  fi

  if [[ "${notarize_dmg}" == "true" ]]; then
    notarize_app
  else
    echo "skipping notarization" >&2
  fi

  # both archives hold the same (stapled) bundle
  local zip="${BUILD_DIR}/Onda-${version}-macos-arm64.zip"
  create_zip "${zip}"
  verify_zip "${zip}" "${version}" "${build}" "${notarize_dmg}"

  local dmg="${BUILD_DIR}/Onda-${version}.dmg"
  create_dmg "${dmg}" "${identity}"
  if [[ "${notarize_dmg}" == "true" ]]; then
    notarize "${dmg}" "${dmg}"
  fi
  verify_dmg "${dmg}" "${notarize_dmg}"
  echo "built ${zip} and ${dmg} (${version} build ${build})"
}

main "$@"
