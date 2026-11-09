#!/bin/sh
# Installs the latest verifique into ~/.local/bin, on macOS or Linux. Windows has install.ps1.
#   curl -fsSL https://raw.githubusercontent.com/smnkgv/verifique/main/install.sh | sh
set -eu

case "$(uname -s)" in
Darwin) os=macos ;;
Linux) os=linux ;;
*)
    echo "This installs verifique on macOS and Linux. On Windows, use install.ps1." >&2
    exit 1
    ;;
esac
case "$(uname -m)" in
arm64 | aarch64) arch=arm64 ;;
x86_64 | amd64) arch=x64 ;;
*)
    echo "There is no build for $(uname -m) processors. Build it from source instead." >&2
    exit 1
    ;;
esac

target=${VERIFIQUE_INSTALL_DIR:-$HOME/.local/bin}
mkdir -p "$target"
curl -fsSL "https://github.com/smnkgv/verifique/releases/latest/download/verifique-$os-$arch.tar.gz" | tar -xz -C "$target"
echo "installed $("$target/verifique" --version) in $target"
case ":$PATH:" in
*":$target:"*) ;;
*) echo "Add $target to your PATH to use it." ;;
esac
echo "Next: start the queue with \`verifique service install\`, add the databases your projects use, such as \`verifique plugin add postgres\`, then give your coding agent the output of \`verifique init\`."
