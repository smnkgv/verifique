# Try verifique on a small shop

A one-page shop with a Search button that does nothing. A real Claude agent opens the shop, presses Search and fails the change, with its reason and a screenshot. You fix the button, and the same check passes. It takes about two minutes.

You need `verifique` running and the Claude CLI signed in. The [main README](../../README.md) covers both.

verifique works on whole repositories, so copy the shop into one of its own:

```bash
cp -R examples/shop ~/verifique-shop
```

```bash
cd ~/verifique-shop && git init -q -b main && git add -A && git commit -qm "A small shop"
```

Ask for the check, and wait for the verdict:

```bash
verifique wait "$(verifique submit --check manual --request "Add a search bar that filters the products when you press Search." --brief "Open the shop, type red shoes into the search box and press Search.")"
```

It fails, because pressing Search changes nothing. Read what the agent saw:

```bash
verifique list
```

```bash
verifique show <job>
```

Now fix the button and check again. You don't need to commit first:

```bash
git apply fix.patch
```

```bash
verifique wait "$(verifique submit --check manual --request "Add a search bar that filters the products when you press Search." --brief "Open the shop, type red shoes into the search box and press Search.")"
```

It passes. To connect your own project, give your coding agent the guide from `verifique init`.
