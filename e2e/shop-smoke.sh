#!/bin/sh
# examples/shop end to end on this OS, with no agent: start the queue, prove the stack with doctor,
# and run a check that passes and one that fails. Uses the binaries in target/release, a home of
# its own and slots 9 and 10, so nothing real is touched.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/verifique-shop-XXXXXX")
export VERIFIQUE_HOME="$WORK/home" VERIFIQUE_NOTIFY=off
export PATH="$ROOT/target/release:$PATH"
SERVER=

finish() {
    if [ -n "$SERVER" ]; then
        verifique stop-server >/dev/null 2>&1 || kill "$SERVER" 2>/dev/null || true
        wait "$SERVER" 2>/dev/null || true
    fi
    rm -rf "$WORK"
}
trap finish EXIT

fail() {
    echo "shop e2e failed: $1" >&2
    [ ! -f "$WORK/server.log" ] || sed 's/^/  server: /' "$WORK/server.log" >&2
    exit 1
}

echo "the shop, in a repository of its own"
cp -R "$ROOT/examples/shop" "$WORK/shop"
cd "$WORK/shop"
# The example's own check is an agent; here a command check stands in for it.
awk '/^checks:/ { exit } { print }' verifique.yaml > verifique.ci.yaml
cat >> verifique.ci.yaml <<'YAML'
checks:
    smoke:
        stack: web
        run: curl -fsS "http://127.0.0.1:$VERIFIQUE_PORT_SHOP/" | grep -q Search
    broken:
        stack: web
        run: curl -fsS "http://127.0.0.1:$VERIFIQUE_PORT_SHOP/missing.html"
YAML
mv verifique.ci.yaml verifique.yaml
git init -q -b main
git add -A
git -c user.name=e2e -c user.email=e2e@example.test commit -qm "A small shop"

echo "the queue"
verifique serve --slots 2 --first-slot 9 >"$WORK/server.log" 2>&1 &
SERVER=$!
tries=0
until verifique status --json | grep -q '"server": true'; do
    tries=$((tries + 1))
    [ "$tries" -lt 150 ] || fail "the queue never came up"
    sleep 0.2
done

echo "doctor"
verifique doctor || fail "doctor"

echo "a check that passes"
verifique wait "$(verifique submit --check smoke)" || fail "the smoke check should pass"

echo "a check that fails"
if verifique wait "$(verifique submit --check broken)"; then
    fail "the broken check should fail"
fi

echo "the shop passed"
