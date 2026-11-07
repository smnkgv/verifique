// The panel as a person sees it: the real page in a real browser, fed the CLI's JSON. The window itself
// is native, so what the Tauri side does with a pin is checked by hand on a Mac.
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const PAGE = new URL("../../panel/ui/index.html", import.meta.url);
const now = new Date().toISOString().slice(0, 19).replace("T", " ");

const job = (id, name, extra = {}) => ({ id, repo: `/src/${name}`, check: "manual", branch: "main", slots: [], priority: false, ...extra });
const running = (id, name, slot) => job(id, name, { slots: [slot], started_at: now, hold_until: null });
const finished = (id, name, status) => job(id, name, { status, finished_at: now });
const quiet = { server: true, slots: 2, low_disk: null, paused: false, needs_you: [], running: [], queued: [], recent: [] };

const browser = await chromium.launch({ channel: "chrome", headless: true });
const page = await browser.newPage({ viewport: { width: 400, height: 720 } });
await page.addInitScript(() => {
    window.sample = null;
    window.pinnedNow = false;
    window.calls = [];
    window.__TAURI__ = {
        core: {
            invoke: async (name, args) => {
                window.calls.push({ name, args });
                if (name === "status") {
                    if (window.sample === null) throw "the queue can't be read";
                    return JSON.stringify(window.sample);
                }
                if (name === "is_pinned") return window.pinnedNow;
                if (name === "set_pinned") window.pinnedNow = args.on;
                return null;
            },
        },
    };
});
await page.goto(fileURLToPath(PAGE).replace(/^/, "file://"));

// A queue that can't be read says so, instead of leaving an empty line.
await page.waitForFunction(() => document.getElementById("panel").innerText.includes("Can't read the queue"));

async function show(state) {
    await page.evaluate(async (next) => {
        window.sample = next;
        window.calls = [];
        await refresh();
    }, { ...quiet, ...state });
}
const text = () => page.locator("#panel").innerText();
const look = () => page.evaluate(() => window.calls.filter((call) => call.name === "show_progress").pop().args);

// A check moves down the panel: Queued, then Running, then Recent, with the next one above Running.
await show({
    running: [running(1, "shop", 4), running(2, "api", 5)],
    queued: [3, 4, 5, 6, 7, 8, 9].map((id) => job(id, `repo${id}`)).concat(job(10, "urgent", { priority: true })),
    recent: [finished(11, "shop", "passed")],
});
const panel = await text();
assert.ok(panel.indexOf("Queued") < panel.indexOf("Running"), "Queued comes first");
assert.ok(panel.indexOf("Running") < panel.indexOf("Recent"), "Recent comes last");
assert.match(panel, /and 3 more/);
assert.ok(panel.indexOf("urgent") > panel.indexOf("repo3"), "the next check sits nearest Running");
assert.ok(panel.indexOf("urgent") < panel.indexOf("Running"));
assert.deepEqual(await look(), { look: "working", count: 10 });

// The tick says what state the queue is in.
await show({});
assert.deepEqual(await look(), { look: "idle", count: 0 });
await show({ recent: [finished(1, "shop", "passed")] });
assert.deepEqual(await look(), { look: "passed", count: 0 });
await show({ recent: [finished(1, "shop", "failed")] });
assert.deepEqual(await look(), { look: "idle", count: 0 });
const needs = [{ repo: "/src/api", branch: "fix", check: "tests", failures: 3, reason: "x" }];
await show({ needs_you: needs, recent: [finished(1, "api", "failed")] });
assert.deepEqual(await look(), { look: "needs", count: 1 });
assert.match(await text(), /Needs you[\s\S]*api \/ fix failed 3 times in a row/);

// Whatever stops the queue says so.
await show({ server: false });
assert.match(await text(), /The queue isn't running/);
await show({ paused: true });
assert.match(await text(), /Paused while verifique doctor runs/);
await show({ low_disk: "Only 3 GB of disk is free, so checks are waiting until 5 GB is" });
assert.match(await text(), /Disk nearly full[\s\S]*Only 3 GB/);

// The pin keeps the panel out, and the page turns see-through.
// A queue that stops being readable says so, instead of showing the last one it read.
await page.evaluate(async () => {
    window.sample = null;
    await refresh();
});
assert.match(await text(), /Can't read the queue/);
assert.equal((await look()).look, "needs");

await show({});
assert.equal(await page.locator("body.pinned").count(), 0);
await page.getByRole("button", { name: "Pin panel" }).click();
await page.waitForSelector("body.pinned");
assert.equal(await page.evaluate(() => window.pinnedNow), true);
const glass = await page.evaluate(() => getComputedStyle(document.getElementById("panel")).backgroundColor);
assert.match(glass, /0\.25\)$/);
await page.getByRole("button", { name: "Unpin panel" }).click();
await page.waitForFunction(() => !document.body.classList.contains("pinned"));

await browser.close();
console.log("panel journey passed");
