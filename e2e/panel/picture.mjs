// Draws the pictures of the status-bar panel on the site and in the README, site/img/menubar-light.png
// and menubar-dark.png: the panel's own page fed a sample queue, as test.mjs feeds it, under the
// macOS menu bar at the top of the pictures as they are, so redrawing keeps that strip.
//   node e2e/panel/picture.mjs
// Run it on a Mac, so the panel is drawn in the system font it uses there.
import { readFileSync, writeFileSync } from "node:fs";
import { chromium } from "playwright-core";

const root = new URL("../../", import.meta.url);
const PAGE = new URL("panel/ui/index.html", root);
const WIDTH = 372;
const BAR = 28;
const PANEL_TOP = 34;
const PANEL_LEFT = 6;
const BOTTOM = 14;
const LOOKS = {
    light: { file: "site/img/menubar-light.png", wallpaper: "linear-gradient(#e1e8ec, #b8c4ca)", shadow: "0 8px 24px rgb(0 0 0 / 0.18)" },
    dark: { file: "site/img/menubar-dark.png", wallpaper: "linear-gradient(#252f37, #101316)", shadow: "0 8px 24px rgb(0 0 0 / 0.5)" },
};

// Three checks queued, two running in slots 4 and 5, and three recent verdicts, which the tick
// counts as 5 left, as the menu bar strip shows.
const ago = (minutes) => new Date(Date.now() - minutes * 60_000).toISOString().slice(0, 19).replace("T", " ");
const job = (id, repo, check, extra = {}) => ({ id, repo: `/src/${repo}`, check, branch: "main", slots: [], priority: false, ...extra });
const sample = {
    server: true,
    slots: 2,
    low_disk: null,
    paused: false,
    needs_you: [],
    // The panel lists the queue from the bottom up, so the first to run is listed last.
    queued: [job(8, "storefront", "manual"), job(7, "billing-api", "tests"), job(6, "docs-site", "e2e")],
    running: [
        job(4, "storefront", "manual", { slots: [4], started_at: ago(2), hold_until: null }),
        job(5, "billing-api", "e2e", { slots: [5], started_at: ago(2), hold_until: null }),
    ],
    recent: [
        job(3, "docs-site", "e2e", { status: "passed", finished_at: ago(1) }),
        job(2, "billing-api", "tests", { status: "passed", finished_at: ago(3) }),
        job(1, "storefront", "manual", { status: "failed", finished_at: ago(4) }),
    ],
};

// Read before anything is drawn, since each picture is redrawn over itself.
const strips = Object.fromEntries(
    Object.entries(LOOKS).map(([look, { file }]) => [look, `data:image/png;base64,${readFileSync(new URL(file, root)).toString("base64")}`]),
);

const browser = await chromium.launch({ channel: process.env.PANEL_BROWSER ?? "chrome", headless: true });
for (const [look, { file, wallpaper, shadow }] of Object.entries(LOOKS)) {
    const page = await browser.newPage({ viewport: { width: WIDTH, height: 900 }, deviceScaleFactor: 2, colorScheme: look });
    await page.addInitScript((queue) => {
        window.__TAURI__ = {
            core: {
                invoke: async (name) => {
                    if (name === "status") return JSON.stringify(queue);
                    if (name === "blurs_behind") return true;
                    if (name === "is_pinned") return false;
                    return null;
                },
            },
        };
    }, sample);
    await page.goto(PAGE.href);
    await page.waitForFunction(() => document.getElementById("panel").innerText.includes("Running at once"));
    const height = await page.evaluate(
        ({ strip, wallpaper, shadow, WIDTH, BAR, PANEL_TOP, PANEL_LEFT, BOTTOM }) => {
            const panel = document.getElementById("panel");
            const body = document.body;
            document.documentElement.style.background = "transparent";
            Object.assign(body.style, { position: "relative", margin: "0", padding: "0", width: `${WIDTH}px`, overflow: "hidden", background: wallpaper });
            Object.assign(panel.style, { position: "absolute", left: `${PANEL_LEFT}px`, top: `${PANEL_TOP}px`, boxShadow: shadow });
            const bar = document.createElement("div");
            Object.assign(bar.style, { position: "absolute", left: "0", top: "0", width: `${WIDTH}px`, height: `${BAR}px`, background: `url(${strip}) 0 0 / ${WIDTH}px auto no-repeat` });
            body.prepend(bar);
            const height = PANEL_TOP + Math.ceil(panel.getBoundingClientRect().height) + BOTTOM;
            body.style.height = `${height}px`;
            return height;
        },
        { strip: strips[look], wallpaper, shadow, WIDTH, BAR, PANEL_TOP, PANEL_LEFT, BOTTOM },
    );
    writeFileSync(new URL(file, root), await page.screenshot({ clip: { x: 0, y: 0, width: WIDTH, height } }));
    console.log(`drew ${file}, ${WIDTH}x${height}`);
    await page.close();
}
await browser.close();
