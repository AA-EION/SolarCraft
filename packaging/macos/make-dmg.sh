#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Wrap the universal SwiftPM binary in SolarCraft.app and build a compressed DMG.
# Usage: make-dmg.sh <universal binary> <version> <build number> <output dir>
set -euo pipefail
BIN=$1; VERSION=$2; BUILD=$3; OUT=$4
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
STAGE=$(mktemp -d)
APP="$STAGE/SolarCraft.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/SolarCraft"
cp "$ROOT/COPYING" "$ROOT/NOTICE" "$APP/Contents/Resources/"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>SolarCraft</string>
  <key>CFBundleDisplayName</key><string>SolarCraft</string>
  <key>CFBundleIdentifier</key><string>org.solarcraft.SolarCraft</string>
  <key>CFBundleExecutable</key><string>SolarCraft</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>es</string><string>de</string><string>pt-BR</string></array>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>GPL-3.0-or-later. See COPYING and NOTICE.</string>
</dict>
</plist>
PLIST
# Ad-hoc signature only: no Developer ID or notarization yet.
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --strict --verbose=2 "$APP"
ln -s /Applications "$STAGE/Applications"
mkdir -p "$OUT"
DMG="$OUT/SolarCraft-$VERSION-macos-universal.dmg"
hdiutil create -volname "SolarCraft $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
rm -rf "$STAGE"
echo "$DMG"
