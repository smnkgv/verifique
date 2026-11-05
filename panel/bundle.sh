#!/bin/sh
# Builds verifique.app. It is signed for this machine only: sharing it needs an Apple Developer ID.
set -e
cd "$(dirname "$0")"
cargo build --release
APP=target/verifique.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/verifique-panel "$APP/Contents/MacOS/verifique"
ICONS=target/icon.iconset
rm -rf "$ICONS" && mkdir -p "$ICONS"
for size in 16 32 128 256 512; do
    sips -z $size $size icons/icon.png --out "$ICONS/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double icons/icon.png --out "$ICONS/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONS" -o "$APP/Contents/Resources/icon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>verifique</string>
    <key>CFBundleDisplayName</key><string>verifique</string>
    <key>CFBundleIdentifier</key><string>dev.verifique.panel</string>
    <key>CFBundleExecutable</key><string>verifique</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>LSUIElement</key><true/>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP"
echo "built $APP"
