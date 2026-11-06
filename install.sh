#!/bin/sh
# Installs the latest verifique into ~/.local/bin.
#   curl -fsSL https://raw.githubusercontent.com/smnkgv/verifique/main/install.sh | sh
set -eu

if [ "$(uname -s)-$(uname -m)" != Darwin-arm64 ]; then
    echo "There is only a build for Macs with Apple Silicon so far. Elsewhere, build it from source." >&2
    exit 1
fi

target=${VERIFIQUE_INSTALL_DIR:-$HOME/.local/bin}
mkdir -p "$target"
curl -fsSL "https://github.com/smnkgv/verifique/releases/latest/download/verifique-macos-arm64.tar.gz" | tar -xz -C "$target"
echo "installed $("$target/verifique" --version) in $target"
case ":$PATH:" in
*":$target:"*) ;;
*) echo "Add $target to your PATH to use it." ;;
esac
echo "Next: start the queue with \`verifique service install\`, then give your coding agent the output of \`verifique init\`."
