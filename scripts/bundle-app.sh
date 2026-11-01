#!/usr/bin/env bash
#
# Builds the Rust core and the Swift shell and assembles a signed
# build/Onda.app (prd/10 REL-1, REL-2). Signs with the "Developer ID
# Application" identity when the keychain has one, so the designated
# requirement (and with it the TCC grants) survives rebuilds; otherwise
# signs ad hoc. Override with SIGNING_IDENTITY. VERSION and BUILD, when set,
# replace the bundle's CFBundleShortVersionString and CFBundleVersion.
#
# Usage: scripts/bundle-app.sh [debug|release]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
readonly SHELL_DIR="${ROOT}/shell/macos"
readonly BUILD_DIR="${ROOT}/build"
readonly APP="${BUILD_DIR}/Onda.app"
readonly ENTITLEMENTS="${SHELL_DIR}/Onda.entitlements"
readonly DEVELOPER_ID="Developer ID Application"
STAGING=""

cleanup() {
  if [[ -n "${STAGING}" ]]; then
    rm -rf "${STAGING}"
  fi
}

signing_identity() {
  if [[ -n "${SIGNING_IDENTITY:-}" ]]; then
    echo "${SIGNING_IDENTITY}"
    return
  fi
  local identities
  identities="$(security find-identity -v -p codesigning)"
  if grep -q "\"${DEVELOPER_ID}" <<<"${identities}"; then
    echo "${DEVELOPER_ID}"
    return
  fi
  echo "-"
}

# kept out of command substitutions, which would ignore a failed build and
# bundle the previous binary. SwiftPM records the deployment target as the
# SDK version, so newer macOS would run Onda in compatibility mode; record
# the SDK actually used instead.
build_shell() {
  local configuration="$1"
  local minimum
  local sdk
  minimum="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' \
    "${SHELL_DIR}/Info.plist")"
  sdk="$(xcrun --show-sdk-version)"
  swift build --package-path "${SHELL_DIR}" -c "${configuration}" \
    --arch arm64 \
    -Xlinker -platform_version -Xlinker macos \
    -Xlinker "${minimum}" -Xlinker "${sdk}"
}

shell_bin_dir() {
  local configuration="$1"
  swift build --package-path "${SHELL_DIR}" -c "${configuration}" \
    --arch arm64 --show-bin-path
}

compile_icon() {
  local resources="$1"
  local iconset="${STAGING}/AppIcon.iconset"
  mkdir -p "${iconset}"
  # the asset catalog's file names already follow the iconset convention
  cp "${ROOT}"/assets/AppIcon.appiconset/icon_*.png "${iconset}/"
  iconutil -c icns "${iconset}" -o "${resources}/AppIcon.icns"
}

# Sparkle (UPD-1) and the media helper's dylib and script (MEDIA-1); the
# executable finds both frameworks through @executable_path/../Frameworks
embed_frameworks() {
  local bin_dir="$1"
  local contents="${APP}/Contents"
  local adapter_script="${bin_dir}/MediaRemoteAdapter_MediaRemoteAdapter.bundle"
  adapter_script+="/Contents/Resources/run.pl"
  mkdir -p "${contents}/Frameworks" "${contents}/Resources/MediaRemoteAdapter"
  ditto "${bin_dir}/Sparkle.framework" \
    "${contents}/Frameworks/Sparkle.framework"
  cp "${bin_dir}/libMediaRemoteAdapter.dylib" "${contents}/Frameworks/"
  cp "${adapter_script}" "${contents}/Resources/MediaRemoteAdapter/run.pl"
}

stamp_version() {
  local plist="$1"
  if [[ -n "${VERSION:-}" ]]; then
    /usr/libexec/PlistBuddy \
      -c "Set :CFBundleShortVersionString ${VERSION}" "${plist}"
  fi
  if [[ -n "${BUILD:-}" ]]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${BUILD}" "${plist}"
  fi
}

assemble() {
  local bin_dir="$1"
  local contents="${APP}/Contents"
  rm -rf "${APP}"
  mkdir -p "${contents}/MacOS" "${contents}/Resources"
  cp "${bin_dir}/Onda" "${contents}/MacOS/Onda"
  cp "${SHELL_DIR}/Info.plist" "${contents}/Info.plist"
  stamp_version "${contents}/Info.plist"
  printf 'APPL????' >"${contents}/PkgInfo"
  cp "${ROOT}"/assets/sounds/*.m4a "${contents}/Resources/"
  compile_icon "${contents}/Resources"
  embed_frameworks "${bin_dir}"
}

sign() {
  local identity="$1"
  local profile="$2"
  local flags=(--force --options runtime --sign "${identity}")
  if [[ "${identity}" != "-" && "${profile}" == "release" ]]; then
    flags+=(--timestamp)
  else
    flags+=(--timestamp=none)
  fi

  # REL-2: nested code first, innermost first, then the executable, then
  # the bundle. Sparkle's helpers follow Sparkle's own signing order; the
  # downloader keeps its entitlements.
  local sparkle="${APP}/Contents/Frameworks/Sparkle.framework"
  local sparkle_version="${sparkle}/Versions/B"
  codesign "${flags[@]}" "${sparkle_version}/XPCServices/Installer.xpc"
  codesign "${flags[@]}" --preserve-metadata=entitlements \
    "${sparkle_version}/XPCServices/Downloader.xpc"
  codesign "${flags[@]}" "${sparkle_version}/Autoupdate"
  codesign "${flags[@]}" "${sparkle_version}/Updater.app"
  codesign "${flags[@]}" "${sparkle}"
  codesign "${flags[@]}" \
    "${APP}/Contents/Frameworks/libMediaRemoteAdapter.dylib"
  codesign "${flags[@]}" --entitlements "${ENTITLEMENTS}" \
    "${APP}/Contents/MacOS/Onda"
  codesign "${flags[@]}" --entitlements "${ENTITLEMENTS}" "${APP}"
  codesign --verify --deep --strict --verbose=2 "${APP}"
}

main() {
  local profile="${1:-debug}"
  case "${profile}" in
    debug | release) ;;
    *)
      echo "usage: $0 [debug|release]" >&2
      exit 2
      ;;
  esac

  STAGING="$(mktemp -d)"
  trap cleanup EXIT

  "${ROOT}/scripts/build-ffi.sh" "${profile}"
  build_shell "${profile}"
  local bin_dir
  bin_dir="$(shell_bin_dir "${profile}")"
  assemble "${bin_dir}"

  local identity
  identity="$(signing_identity)"
  if [[ "${identity}" == "-" ]]; then
    echo "no ${DEVELOPER_ID} identity; signing ad hoc" >&2
  fi
  sign "${identity}" "${profile}"
  # ad hoc signatures print it commented out: "# designated => cdhash ..."
  codesign --display --requirements - "${APP}" 2>&1 \
    | grep -E '^(# )?designated' >&2
  echo "built ${APP} (${profile})"
}

main "$@"
