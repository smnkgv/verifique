// Renders a film page frame by frame. The page must set window.DURATION (seconds) and window.seek(t).
//
//   node render.mjs <page.html> <outdir> [--name film] [--fps 30] [--size 1920x1080]
//                   [--query cut=1x1] [--still 2.5,6.2] [--no-video]
//
// Exits 1 when the page reported any error, so a run with errors cannot pass.
import { spawn } from "node:child_process";
import { mkdirSync, readFile } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const [pageArg, outdir, ...rest] = process.argv.slice(2);
if (!pageArg || !outdir) {
    console.error("usage: node render.mjs <page.html> <outdir> [--name n] [--fps n] [--size WxH] [--query q] [--still t,…] [--no-video]");
    process.exit(2);
}

const option = (name) => {
    const at = rest.indexOf(name);
    return at >= 0 ? rest[at + 1] : undefined;
};
const fps = Number(option("--fps") ?? 30);
const name = option("--name") ?? "film";
const query = option("--query") ?? "";
const stills = (option("--still") ?? "").split(",").filter(Boolean).map(Number);
const [width, height] = (option("--size") ?? "1920x1080").split("x").map(Number);
const video = !rest.includes("--no-video");

const TYPES = { ".html": "text/html", ".woff2": "font/woff2" };
const server = createServer((request, response) => {
    const file = path.join(ROOT, decodeURIComponent(new URL(request.url, "http://x").pathname));
    if (!file.startsWith(ROOT)) return response.writeHead(403).end();
    readFile(file, (error, body) => {
        if (error) return response.writeHead(404).end();
        response.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream" }).end(body);
    });
});
await new Promise((ready) => server.listen(0, "127.0.0.1", ready));

const relative = path.relative(ROOT, path.resolve(pageArg)).split(path.sep).join("/");
const url = `http://127.0.0.1:${server.address().port}/${relative}${query ? `?${query}` : ""}`;

mkdirSync(outdir, { recursive: true });
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });

const errors = [];
page.on("pageerror", (error) => errors.push(String(error)));
page.on("console", (message) => message.type() === "error" && errors.push(message.text()));
page.on("requestfailed", (request) => errors.push(`failed to load ${request.url()}`));
page.on("response", (response) => response.status() >= 400 && errors.push(`${response.status()} ${response.url()}`));

await page.goto(url);
await page.waitForLoadState("networkidle");
await page.evaluate(() => document.fonts.ready);
const duration = await page.evaluate(() => window.DURATION);

for (const t of stills) {
    await page.evaluate((at) => window.seek(at), t);
    await page.screenshot({ path: path.join(outdir, `still-${String(t).replace(".", "_")}.png`) });
}

let frames = 0;
const out = path.join(outdir, `${name}.mp4`);
if (video) {
    const ffmpeg = spawn(
        "ffmpeg",
        ["-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", String(fps), "-i", "-", "-c:v", "libx264",
            "-profile:v", "high", "-pix_fmt", "yuv420p", "-crf", "18", "-movflags", "+faststart", out],
        { stdio: ["pipe", "inherit", "inherit"] },
    );
    frames = Math.round(duration * fps);
    for (let i = 0; i < frames; i++) {
        await page.evaluate((at) => window.seek(at), i / fps);
        const shot = await page.screenshot({ type: "png" });
        if (!ffmpeg.stdin.write(shot)) await new Promise((drained) => ffmpeg.stdin.once("drain", drained));
    }
    ffmpeg.stdin.end();
    await new Promise((done, failed) =>
        ffmpeg.on("close", (code) => (code === 0 ? done() : failed(new Error(`ffmpeg exited with ${code}`)))));
}

await browser.close();
server.close();
console.log(JSON.stringify({ frames, fps, duration, size: `${width}x${height}`, query, out: video ? out : null, stills: stills.length, errors }));
process.exit(errors.length ? 1 : 0);
