import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, readFile, readdir, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { chromium } from "playwright";

const origin = "http://127.0.0.1:4322";
const output = resolve("dist");
const files = (await readdir(output, { recursive: true })).filter((file) => file.endsWith(".html"));
assert(files.includes("docs/index.html"), "Build the site before running this check.");
for (const file of files) {
  const html = await readFile(resolve(output, file), "utf8");
  const pageUrl = new URL(file.replace(/index\.html$/, ""), `${origin}/`);
  for (const [, href] of html.matchAll(/href="([^"]+)"/g)) {
    const url = new URL(href.replaceAll("&amp;", "&"), pageUrl);
    if (url.origin !== origin) continue;
    let target = resolve(output, `.${decodeURIComponent(url.pathname)}`);
    const info = await stat(target).catch(() => null);
    assert(info, `Broken link in ${file}: ${href}`);
    if (info.isDirectory()) target = resolve(target, "index.html");
    const targetFile = await readFile(target, "utf8");
    if (url.hash && target.endsWith(".html")) {
      assert(
        targetFile.includes(`id="${decodeURIComponent(url.hash.slice(1))}"`),
        `Broken anchor in ${file}: ${href}`,
      );
    }
  }
}

await mkdir("test-results", { recursive: true });
const server = spawn(
  process.execPath,
  ["node_modules/astro/bin/astro.mjs", "preview", "--host", "127.0.0.1", "--port", "4322"],
  { stdio: "pipe" },
);
let serverOutput = "";
server.stdout.on("data", (data) => {
  serverOutput += data;
});
server.stderr.on("data", (data) => {
  serverOutput += data;
});
let browser;
try {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (server.exitCode !== null) throw new Error(serverOutput);
    if (
      await fetch(origin)
        .then((response) => response.ok)
        .catch(() => false)
    )
      break;
    if (attempt === 99) throw new Error(`Preview did not start: ${serverOutput}`);
    await delay(100);
  }
  browser = await chromium.launch();
  const errors = [];
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(origin);
  await page.waitForSelector(".story.enhanced");
  await page.evaluate(() => document.fonts.ready);
  assert.match(await page.locator("h1").innerText(), /Your schema leads\.\s*Rust delivers\./);
  assert.equal(await page.locator(".closing").count(), 0);
  assert.equal(await page.locator(".phase-indicators button").count(), 0);
  await page.screenshot({ path: "test-results/hero.png" });
  const scrollToProgress = async (progress, phase) => {
    await page
      .locator(".story")
      .evaluate(
        (element, p) =>
          window.scrollTo(
            0,
            window.scrollY +
              element.getBoundingClientRect().top +
              p * (element.offsetHeight - window.innerHeight),
          ),
        progress,
      );
    await page.waitForFunction(
      (expected) =>
        document.querySelector(".story").dataset.phase === expected &&
        document.body.dataset.tone === "dark",
      phase,
    );
  };
  await scrollToProgress(0.1, "initial");
  assert.equal(await page.locator("body").getAttribute("data-tone"), "dark");
  await page.screenshot({ path: "test-results/schema.png" });
  assert.match(await page.locator('[aria-current="step"]').innerText(), /Define/);
  await scrollToProgress(0.45, "schema");
  assert.match(await page.locator('[aria-current="step"]').innerText(), /Evolve/);
  assert.match(await page.locator('[data-code="schema-after"]').innerText(), /version: String!/);
  assert.equal(
    await page.locator('[data-code="resolver-after"]').count(),
    0,
    "Resolver changes only after the build step.",
  );
  await scrollToProgress(0.8, "synced");
  assert.match(await page.locator('[aria-current="step"]').innerText(), /Build/);
  await page.evaluate(() =>
    Promise.all(document.getAnimations().map((animation) => animation.finished)),
  );
  const resolver = await page.locator('[data-code="resolver-after"]').innerText();
  assert.match(resolver, /async fn version/);
  assert.match(resolver, /unimplemented!\(\)/);
  assert.match(resolver, /Ok\(format!\("Hello, \{\}", args.name\)\)/);
  await page.screenshot({ path: "test-results/evolved.png" });
  await scrollToProgress(0.1, "initial");
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForFunction(() => document.body.dataset.tone === "light");
  await page.keyboard.press("Tab");
  assert.equal(await page.locator(":focus").innerText(), "Skip to content");

  await page.setViewportSize({ width: 1280, height: 800 });
  await scrollToProgress(0.8, "synced");
  assert.match(await page.locator('[aria-current="step"]').innerText(), /Build/);
  assert(
    await page.locator(".story-stage").evaluate((element) => element.offsetHeight <= innerHeight),
    "Pinned scene must fit a laptop viewport.",
  );

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(origin);
  await page.waitForSelector(".story.enhanced");
  assert(
    await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
    "Mobile page must not overflow horizontally.",
  );
  await page.screenshot({ path: "test-results/mobile-hero.png" });
  await page.locator(".story").evaluate((element) => element.scrollIntoView());
  await page.waitForFunction(() => document.querySelector(".story").dataset.phase === "synced");
  assert.match(await page.locator('[data-code="resolver-after"]').innerText(), /async fn version/);
  await page.screenshot({ path: "test-results/mobile-evolved.png", fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(origin);
  await page.waitForSelector(".story.enhanced");
  assert.equal(
    await page.locator(".story-stage").evaluate((element) => getComputedStyle(element).position),
    "static",
  );
  await page.locator(".story").evaluate((element) => element.scrollIntoView());
  await page.waitForFunction(() => document.querySelector(".story").dataset.phase === "synced");
  assert.equal(await page.locator(".story").getAttribute("data-phase"), "synced");

  await page.goto(`${origin}/docs/`);
  assert.equal(await page.locator("h1").innerText(), "Introduction");
  await page.goto(`${origin}/docs/architecture/`);
  assert.match(await page.locator("h1").innerText(), /architecture/);
  await page.screenshot({ path: "test-results/docs.png" });
  const noJS = await browser.newPage({ javaScriptEnabled: false });
  await noJS.goto(origin);
  assert.equal(await noJS.locator(".code-version:visible").count(), 4);
  assert.equal(await noJS.locator(".phase-indicators:visible").count(), 0);
  assert.deepEqual(errors, [], "No browser runtime errors.");
  console.log(
    `Checked ${files.length} HTML pages and local links; scroll, reverse scroll, mobile, reduced motion, no-JS, and documentation passed.`,
  );
} finally {
  await browser?.close();
  if (server.exitCode === null) {
    const exited = once(server, "exit");
    server.kill();
    await exited;
  }
}
