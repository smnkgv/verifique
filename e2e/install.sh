#!/bin/sh
# A stranger with a fresh clone: the README's build steps, the service, a first project, a first check.
# Everything lives in a throwaway home, and it has a service of its own, so nothing real is touched.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/verifique-install-XXXXXX")
FIRST_SLOT=${E2E_FIRST_SLOT:-9}
CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}
export CARGO_HOME RUSTUP_HOME
export HOME="$WORK/home" VERIFIQUE_HOME="$WORK/home/.verifique-e2e"
export PATH="$HOME/.local/bin:$PATH"
SHELL=/bin/sh
export SHELL
mkdir -p "$HOME/.local/bin"
SERVER=

finish() {
    [ "${SERVICE:-no}" = no ] || verifique service uninstall >/dev/null 2>&1 || true
    [ -z "$SERVER" ] || kill "$SERVER" 2>/dev/null || true
    rm -rf "$WORK"
}
trap finish EXIT

fail() {
    echo "e2e failed: $1" >&2
    exit 1
}

echo "fresh clone"
git clone -q "$ROOT" "$WORK/verifique"
cd "$WORK/verifique"

echo "the README's build steps"
# The block that runs the tests is left to the behaviour tier.
python3 - <<'PY' | sh -eu
import re
blocks = re.findall(r"```bash\n(.*?)```", open("README.md").read().split("## Build")[1], re.S)
print("\n".join(line for block in blocks for line in block.splitlines() if line != "cargo test"))
PY
[ "$(command -v verifique)" = "$HOME/.local/bin/verifique" ] || fail "verifique is not on the PATH"
verifique --version

echo "the guide and the first status"
verifique init | grep -q "Connect this project" || fail "init printed no guide"
verifique init --rules | grep -q "needs-you" || fail "init printed no rules"
verifique status | grep -q "no verifique server is running" || fail "status didn't say the queue is down"

echo "the service"
# Installing a service belongs on a clean machine, so it only happens under CI.
if [ "${CI:-}" = true ] && { [ "$(uname)" = Darwin ] || systemctl --user show-environment >/dev/null 2>&1; }; then
    SERVICE=yes
    verifique service install --slots 2 --first-slot "$FIRST_SLOT" | grep -q "installed" || fail "service install"
else
    SERVICE=no
    echo "not on a clean machine, so the server runs by hand"
    verifique serve --slots 2 --first-slot "$FIRST_SLOT" >"$WORK/server.log" 2>&1 &
    SERVER=$!
fi
up() {
    tries=0
    until verifique status --json | grep -q '"server": true'; do
        tries=$((tries + 1))
        [ "$tries" -lt 100 ] || fail "the queue never came up"
        sleep 0.2
    done
}
up

echo "a first project"
cp -R "$ROOT/e2e/sample" "$WORK/first-project"
cd "$WORK/first-project"
git init -q -b main
git add -A
git -c user.name=e2e -c user.email=e2e@example.test commit -qm "A first project"
verifique doctor | grep -q "doctor passed" || fail "doctor"

echo "a first check, passing and failing"
verifique wait "$(verifique submit --check fast)" >/dev/null || fail "the fast check should pass"
verifique verdict --check fast >/dev/null || fail "verdict should report the pass"
job=$(verifique submit --check isolation)
verifique wait "$job" >/dev/null || fail "the stack check should pass"
echo bad >behaviour.txt
if job=$(verifique submit --check isolation) && verifique wait "$job" >/dev/null; then
    fail "the bad change should fail"
fi
verifique show "$job" | grep -q "planted bug" || fail "the verdict should say why"

if [ "$SERVICE" = yes ]; then
    echo "restart and removal"
    verifique service restart >/dev/null
    up
    verifique service uninstall >/dev/null
    sleep 1
    if verifique status --json | grep -q '"server": true' && sleep 16 && verifique status --json | grep -q '"server": true'; then
        fail "the service survived its removal"
    fi
fi

echo "install journey passed"
