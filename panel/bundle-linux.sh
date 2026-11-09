#!/bin/sh
# Builds the panel for Linux: target/verifique-panel, and target/verifique-panel.desktop, which starts
# it from the app menu or at login. Building needs WebKitGTK and an AppIndicator library.
set -e
cd "$(dirname "$0")"
cargo build --release
# Removed first, so a panel that's running keeps its copy and the new one takes its place.
rm -f target/verifique-panel
mkdir -p target
cp "${CARGO_TARGET_DIR:-target}/release/verifique-panel" target/verifique-panel
cat > target/verifique-panel.desktop <<DESKTOP
[Desktop Entry]
Type=Application
Name=verifique
Comment=The verifique queue in the tray
Exec="$PWD/target/verifique-panel"
Icon=$PWD/icons/icon.png
Terminal=false
Categories=Development;
DESKTOP
echo "built target/verifique-panel and target/verifique-panel.desktop"
