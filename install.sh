#!/bin/sh
# Installs the latest verifique into ~/.local/bin.
#   curl -fsSL https://raw.githubusercontent.com/smnkgv/verifique/main/install.sh | sh
set -eu

case "$(uname -s)-$(uname -m)" in
Darwin-arm64) name=macos-arm64 ;;
Darwin-x86_64) name=macos-x86_64 ;;
Linux-x86_64) name=linux-x86_64 ;;
Linux-aarch64 | Linux-arm64) name=linux-arm64 ;;
*)
    echo "There is no verifique build for $(uname -s) on $(uname -m) yet." >&2
    exit 1
    ;;
esac

target=${VERIFIQUE_INSTALL_DIR:-$HOME/.local/bin}
mkdir -p "$target"
curl -fsSL "https://github.com/smnkgv/verifique/releases/latest/download/verifique-$name.tar.gz" | tar -xz -C "$target"
echo "installed $("$target/verifique" --version) in $target"
case ":$PATH:" in
*":$target:"*) ;;
*) echo "Add $target to your PATH to use it." ;;
esac
echo "Next: start the queue with \`verifique service install\`, then give your coding agent the output of \`verifique init\`."
