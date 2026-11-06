# Testing from the outside

These tests treat verifique as a stranger or an agent does: the real binary, real repositories, real databases and, at the top, a real agent.

| Tier | What it proves | Run it | Cost |
|---|---|---|---|
| Behaviour | The queue, verdicts and isolation, through the CLI | `cargo test` | Free, 30 s |
| Install | A fresh clone builds, connects a project and checks it, following the README | `sh e2e/install.sh` | Free, 2 min |
| Many agents | 15 sessions at once, while the queue is killed and resized | `python3 e2e/many_agents.py` | Free, 2 min |
| Panel | The real page in a real browser, fed the CLI's JSON | `node e2e/panel/test.mjs` | Free, 5 s |
| The shop | A real agent fails the broken button and passes the fix, from the example's README | `python3 e2e/shop.py` | About 50 cents |
| Scoreboard | A real agent catches planted bugs and passes good changes | `python3 e2e/shop_cases.py` | About a dollar |
| Blind setup | A fresh agent connects a project with nothing but `verifique init` | `sh e2e/blind.sh <copy>` | $5 to $10 |

`sh e2e/run.sh` runs the free tiers, and `sh e2e/run.sh real` adds the shop and the scoreboard. To run the free tiers on a clean Linux machine:

```bash
docker build -f e2e/Dockerfile -t verifique-e2e . && docker run --rm verifique-e2e
```

Each journey runs a queue of its own in a temporary folder on slots 9 and 10. The install journey only installs a real service with `E2E_SERVICE=yes`, on a clean machine. Use `E2E_FIRST_SLOT` if those slots are taken, and keep it under 16 so every slot has a Redis database.

Every bug found by hand becomes a test here. To score a private project, keep its cases outside this repository and point `e2e/scoreboard.py` at them. [`verifique.yaml`](../verifique.yaml) lets verifique check itself.

Windows isn't supported yet, so it has no tier.
