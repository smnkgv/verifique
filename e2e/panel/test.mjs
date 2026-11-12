// The panel as a person sees it: the real page in a real browser, fed the CLI's JSON. The window and the
// tray icon are native, so what the Tauri side does with them is checked by hand on each OS.
// The browser is the installed Chrome, or Edge on Windows, whose engine is the one the panel runs in
// there. PANEL_BROWSER names another Playwright channel.
import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const PAGE = new URL("../../panel/ui/index.html", import.meta.url);
const now = new Date().toISOString().slice(0, 19).replace("T", " ");

const job = (id, name, extra = {}) => ({ id, repo: `/src/${name}`, check: "manual", branch: "main", slots: [], priority: false, ...extra });
const running = (id, name, slot) => job(id, name, { slots: [slot], started_at: now, hold_until: null });
const finished = (id, name, status) => job(id, name, { status, finished_at: now });
const quiet = { server: true, slots: 2, low_disk: null, paused: false, needs_you: [], running: [], queued: [], recent: [] };

const channel = process.env.PANEL_BROWSER ?? (process.platform === "win32" ? "msedge" : "chrome");
const browser = await chromium.launch({ channel, headless: true }).catch((error) => {
    console.error(`The panel journey needs ${channel} installed, or PANEL_BROWSER set to a browser that is.`);
    throw error;
});
const page = await browser.newPage({ viewport: { width: 400, height: 720 } });
await page.addInitScript(() => {
    window.sample = null;
    window.pinnedNow = false;
    window.notice = null;
    window.updateFails = null;
    window.calls = [];
    window.__TAURI__ = {
        core: {
            invoke: async (name, args) => {
                window.calls.push({ name, args });
                if (name === "status") {
                    if (window.sample === null) throw "the queue can't be read";
                    return JSON.stringify(window.sample);
                }
                if (name === "updates") return JSON.stringify({ notice: window.notice });
                if (name === "update") {
                    if (window.updateFails) throw window.updateFails;
                    return "up to date: verifique 0.3.4";
                }
                if (name === "is_pinned") return window.pinnedNow;
                if (name === "set_pinned") window.pinnedNow = args.on;
                if (name === "blurs_behind") return !location.search.includes("unblurred");
                return null;
            },
        },
    };
});
await page.goto(PAGE.href);

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
assert.deepEqual(await look(), { look: "working", count: 10, update: "" });

// A repository on Windows is named by its folder too.
await show({ running: [{ ...running(1, "x", 4), repo: "C:\\src\\shop" }] });
assert.match(await text(), /shop · testing · manual/);
assert.doesNotMatch(await text(), /src/);

// How many copies run at once, and what each is for, in words anyone reads the same way.
await show({ slots: 3, running: [running(1, "shop", 4), { ...running(2, "api", 5), check: "stack" }] });
const counts = await text();
assert.match(counts, /2 of 3 running/);
assert.match(counts, /shop · testing · manual/);
assert.match(counts, /api · kept running for you/);
assert.match(counts, /Running at once/);

// The tick says what state the queue is in.
await show({});
assert.deepEqual(await look(), { look: "idle", count: 0, update: "" });
await show({ recent: [finished(1, "shop", "passed")] });
assert.deepEqual(await look(), { look: "passed", count: 0, update: "" });
await show({ recent: [finished(1, "shop", "failed")] });
assert.deepEqual(await look(), { look: "idle", count: 0, update: "" });
const needs = [{ repo: "/src/api", branch: "fix", check: "tests", failures: 3, reason: "x" }];
await show({ needs_you: needs, recent: [finished(1, "api", "failed")] });
assert.deepEqual(await look(), { look: "needs", count: 1, update: "" });
assert.match(await text(), /Needs you[\s\S]*api \/ fix failed 3 times in a row/);

// Whatever stops the queue says so.
await show({ server: false });
assert.match(await text(), /The queue isn't running/);
await show({ paused: true });
assert.match(await text(), /Paused while verifique doctor runs/);
await show({ low_disk: "Only 3 GB of disk is free, so checks are waiting until 5 GB is" });
assert.match(await text(), /Disk nearly full[\s\S]*Only 3 GB/);

// A newer verifique is one line with an Update button, and a blue dot on the tick.
async function tell(notice) {
    await page.evaluate(async (next) => {
        window.notice = next;
        await refreshUpdates();
    }, notice);
}
const release = {
    kind: "release",
    line: "verifique 0.3.4 is out (this is 0.3.3): run `verifique update`, which restarts the queue",
    short: "verifique 0.3.4 is out",
    updatable: true,
    restarts: true,
    url: "https://github.com/smnkgv/verifique/releases/tag/v0.3.4",
};
const updated = () => page.evaluate(() => window.calls.filter((call) => call.name === "update").length);
await show({ running: [running(1, "shop", 4), { ...running(2, "api", 5), check: "stack" }] });
await tell(release);
assert.match(await text(), /verifique 0\.3\.4 is out/);
assert.deepEqual(await look(), { look: "working", count: 2, update: "verifique 0.3.4 is out" });

// Updating restarts the queue, so with checks running it first says what that stops.
await page.getByRole("button", { name: "Update", exact: true }).click();
assert.match(await text(), /Updating restarts the queue: 1 started stack stops, and 1 running check starts again from the beginning\./);
assert.equal(await updated(), 0);
await page.getByRole("button", { name: "Not now" }).click();
assert.doesNotMatch(await text(), /Updating restarts/);
assert.equal(await updated(), 0);
await page.getByRole("button", { name: "Update", exact: true }).click();
await page.getByRole("button", { name: "Update now" }).click();
await page.waitForFunction(() => document.getElementById("panel").innerText.includes("Updated."));
assert.equal(await updated(), 1);

// With nothing running it updates at once, and a failure stays with its reason.
await show({});
await page.evaluate(() => {
    window.updateFails = "brew: no bottle for this machine";
});
await page.getByRole("button", { name: "Update", exact: true }).click();
await page.waitForFunction(() => document.getElementById("panel").innerText.includes("no bottle"));
assert.equal(await updated(), 1);
assert.doesNotMatch(await text(), /Updating restarts/);
await page.evaluate(() => {
    window.updateFails = null;
});

// A build from source is updated by hand, so it gets what the release changed instead of a button.
await tell({ ...release, line: "verifique 0.3.4 is out (this is 0.3.3): pull and build again in /src/verifique", updatable: false });
assert.equal(await page.getByRole("button", { name: "Update", exact: true }).count(), 0);
await page.getByRole("button", { name: "What's new" }).click();
const opened = await page.evaluate(() => window.calls.filter((call) => call.name === "open_release").pop()?.args);
assert.deepEqual(opened, { url: release.url });

// Once there is nothing to update, the line and the dot go.
await tell(null);
assert.doesNotMatch(await text(), /is out/);
assert.deepEqual(await look(), { look: "idle", count: 0, update: "" });

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

// Where the OS blurs nothing behind it, as on Windows and Linux, a pinned panel lets less through.
await page.goto(`${PAGE.href}?unblurred`);
await page.waitForFunction(() => document.body.classList.contains("unblurred"));
await show({});
await page.getByRole("button", { name: "Pin panel" }).click();
await page.waitForSelector("body.pinned");
const frosted = await page.evaluate(() => getComputedStyle(document.getElementById("panel")).backgroundColor);
assert.match(frosted, /0\.85\)$/);

await browser.close();
console.log("panel journey passed");
