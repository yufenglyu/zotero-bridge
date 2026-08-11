#!/usr/bin/env bash
#
# Zotero Bridge - macOS build & package script
#
# Usage (from repo root):
#   bash scripts/release-macos.sh
#   bash scripts/release-macos.sh --skip-build
#   bash scripts/release-macos.sh --sign
#
# Artifacts go to target/dist/:
#   zotero-bridge-v<version>-macos-universal-app.zip
#   zotero-bridge-v<version>-macos-universal.dmg

set -euo pipefail

SKIP_BUILD=0
SIGN=0

for arg in "$@"; do
  case "$arg" in
    --skip-build)
      SKIP_BUILD=1
      ;;
    --sign)
      SIGN=1
      ;;
    -h|--help)
      sed -n '1,18p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(node -e "console.log(require('./apps/desktop/src-tauri/tauri.conf.json').version)")"
echo "==> Version: v${VERSION}"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "==> Ensuring macOS Rust targets"
  rustup target add aarch64-apple-darwin x86_64-apple-darwin

  echo "==> Building frontend"
  (cd apps/desktop && npm run build)

  echo "==> Building macOS universal app"
  TAURI_ARGS=(build --target universal-apple-darwin --bundles app,dmg --ci)
  if [[ "$SIGN" -eq 0 ]]; then
    TAURI_ARGS+=(--no-sign)
  fi
  (cd apps/desktop && npm run tauri -- "${TAURI_ARGS[@]}")
fi

BUNDLE_ROOT="$ROOT/target/universal-apple-darwin/release/bundle"
DIST="$ROOT/target/dist"
mkdir -p "$DIST"

APP_PATH="$(find "$BUNDLE_ROOT/macos" -maxdepth 1 -type d -name "*.app" | head -1 || true)"
DMG_PATH="$(find "$BUNDLE_ROOT/dmg" -maxdepth 1 -type f -name "*.dmg" | head -1 || true)"

if [[ -z "$APP_PATH" ]]; then
  echo "macOS .app bundle not found under $BUNDLE_ROOT/macos" >&2
  exit 1
fi
if [[ -z "$DMG_PATH" ]]; then
  echo "macOS .dmg bundle not found under $BUNDLE_ROOT/dmg" >&2
  exit 1
fi

APP_ZIP="$DIST/zotero-bridge-v${VERSION}-macos-universal-app.zip"
DMG_OUT="$DIST/zotero-bridge-v${VERSION}-macos-universal.dmg"
rm -f "$APP_ZIP" "$DMG_OUT"

echo "==> Assembling macOS artifacts"
ditto -c -k --keepParent "$APP_PATH" "$APP_ZIP"
cp "$DMG_PATH" "$DMG_OUT"

echo ""
echo "Done. Artifacts:"
printf '  target/dist/%s\n' "$(basename "$APP_ZIP")"
printf '  target/dist/%s\n' "$(basename "$DMG_OUT")"
