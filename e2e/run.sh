#!/bin/sh
# Everything from the outside. `sh e2e/run.sh` runs what is free, and `sh e2e/run.sh real` adds the
# journeys where a real agent checks, which need Claude signed in and cost about a dollar.
set -eu
cd "$(dirname "$0")/.."

cargo build --release
echo "== behaviour"
cargo test
echo "== install"
sh e2e/install.sh
echo "== many agents"
python3 e2e/many_agents.py
echo "== the shop, no agent"
sh e2e/shop-smoke.sh
if command -v node >/dev/null; then
    echo "== panel"
    (cd e2e/panel && npm ci --silent --no-audit --no-fund && node test.mjs)
else
    echo "== panel skipped: it needs node, and Chrome, or Edge on Windows"
fi
if [ "${1:-}" = real ]; then
    echo "== the shop, with a real agent"
    python3 e2e/shop.py
    echo "== the scoreboard"
    python3 e2e/shop_cases.py
fi
echo "every tier passed"
