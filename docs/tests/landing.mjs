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
assert(!files.some((file) => file.startsWith("archives/")), "Archives must not be published.");
for (const file of files) {
  const html = await readFile(resolve(output, file), "utf8");
  assert(!/href="[^"]*\/archives\//.test(html), `Archive link in published page: ${file}`);
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
  await page.keyboard.press("Tab");
  assert.equal(await page.locator(":focus").innerText(), "Skip to content");
  assert.equal(await page.locator(".closing").count(), 0);
  assert.equal(await page.locator(".phase-indicators button").count(), 3);
  await page.screenshot({ path: "test-results/hero.png" });
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.getByRole("button", { name: "Copy installation command" }).click();
  assert.equal(
    await page.evaluate(() => navigator.clipboard.readText()),
    "cargo install --git https://github.com/Necrass-Dev/NecrassRs.git necrassrs-cli --locked",
  );
  const expectPhase = async (phase) => {
    await page.waitForFunction(
      (expected) => document.querySelector(".story").dataset.phase === expected,
      phase,
    );
  };
  const scrollTimeline = async (progress) => {
    await page.locator(".pin-spacer").evaluate((spacer, value) => {
      const story = spacer.querySelector(".story");
      const start = scrollY + spacer.getBoundingClientRect().top;
      scrollTo(0, start + value * (spacer.offsetHeight - story.offsetHeight));
    }, progress);
  };
  const expectOpacity = async (code, opacity) => {
    await page.waitForFunction(
      ([name, expected]) =>
        Math.abs(
          Number(getComputedStyle(document.querySelector(`[data-code="${name}"]`)).opacity) -
            expected,
        ) < 0.04,
      [code, opacity],
    );
  };
  await page.waitForSelector(".pin-spacer");
  await page
    .locator(".pin-spacer")
    .evaluate((element) => scrollTo(0, scrollY + element.getBoundingClientRect().top - 200));
  await expectPhase("initial");
  await expectOpacity("schema-after", 0);
  await scrollTimeline(0.1);
  await expectPhase("initial");
  await page.waitForFunction(() => document.body.dataset.tone === "dark");
  await page.screenshot({ path: "test-results/schema.png" });
  const pinnedTop = await page
    .locator(".story")
    .evaluate((element) => element.getBoundingClientRect().top);
  const oldScroll = await page.evaluate(() => scrollY);
  await page.mouse.move(720, 500);
  await page.mouse.wheel(0, 80);
  await page.waitForFunction((y) => scrollY > y, oldScroll);
  assert(
    Math.abs(
      (await page.locator(".story").evaluate((element) => element.getBoundingClientRect().top)) -
        pinnedTop,
    ) < 2,
    "Native scrolling must keep the scene pinned.",
  );
  // A halfway fade demonstrates continuous scrubbing instead of discrete DOM swaps.
  await scrollTimeline(0.36);
  await expectOpacity("schema-after", 0.5);
  await page.screenshot({ path: "test-results/transition.png" });
  await scrollTimeline(0.52);
  await expectPhase("schema");
  await expectOpacity("schema-after", 1);
  assert.match(await page.locator('[aria-current="step"]').innerText(), /Evolve/);
  assert.match(await page.locator('[data-code="schema-after"]').innerText(), /version: String!/);
  await expectOpacity("resolver-after", 0);
  await scrollTimeline(0.9);
  await expectPhase("synced");
  await expectOpacity("resolver-after", 1);
  assert.match(await page.locator('[aria-current="step"]').innerText(), /Build/);
  const resolver = await page.locator('[data-code="resolver-after"]').innerText();
  assert.match(resolver, /async fn version/);
  assert.match(resolver, /unimplemented!\(\)/);
  assert.match(resolver, /Ok\(format!\("Hello, \{\}", args.name\)\)/);
  assert(
    Math.abs(
      (await page.locator(".story").evaluate((element) => element.getBoundingClientRect().top)) -
        pinnedTop,
    ) < 2,
  );
  await page.screenshot({ path: "test-results/evolved.png" });
  await page.getByRole("button", { name: "02 Evolve" }).click();
  await expectPhase("schema");
  await expectOpacity("resolver-after", 0);
  await page.getByRole("button", { name: "01 Define" }).click();
  await expectPhase("initial");
  await expectOpacity("schema-after", 0);
  await page.getByRole("button", { name: "03 Build" }).click();
  await expectPhase("synced");
  await expectOpacity("resolver-after", 1);
  await scrollTimeline(0.52);
  await expectPhase("schema");
  await expectOpacity("resolver-after", 0);
  await scrollTimeline(0.1);
  await expectPhase("initial");
  await expectOpacity("schema-after", 0);
  await page.evaluate(() => scrollTo(0, 0));
  await page.waitForFunction(() => document.body.dataset.tone === "light");

  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto(origin);
  await page.waitForSelector(".pin-spacer");
  await page
    .locator(".pin-spacer")
    .evaluate((element) => scrollTo(0, scrollY + element.getBoundingClientRect().top - 80));
  await delay(500);
  assert.equal(
    await page.locator(".story").getAttribute("data-phase"),
    "initial",
    "Short desktop screens must wait until the scene fully enters.",
  );
  assert.equal(
    await page
      .locator('[data-code="schema-after"]')
      .evaluate((element) => Number(getComputedStyle(element).opacity)),
    0,
  );
  await scrollTimeline(0.9);
  await expectPhase("synced");
  assert(
    await page.locator(".story-stage").evaluate((element) => element.offsetHeight <= innerHeight),
    "The pinned scene must fit a laptop viewport.",
  );
  assert.equal(await page.locator(".pin-spacer").count(), 1);

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(origin);
  await page.waitForSelector(".story.enhanced");
  assert.equal(await page.locator(".pin-spacer").count(), 0);
  assert(
    await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
    "Mobile page must not overflow horizontally.",
  );
  await page.screenshot({ path: "test-results/mobile-hero.png" });
  await page
    .locator(".story")
    .evaluate((element) => scrollTo(0, scrollY + element.getBoundingClientRect().top - 80));
  await delay(500);
  assert.equal(
    await page.locator(".story").getAttribute("data-phase"),
    "initial",
    "Mobile screens must wait until the scene fully enters.",
  );
  assert.equal(
    await page
      .locator('[data-code="schema-after"]')
      .evaluate((element) => Number(getComputedStyle(element).opacity)),
    0,
  );
  await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
  await expectPhase("synced");
  await expectOpacity("resolver-after", 1);
  await page.screenshot({ path: "test-results/mobile-evolved.png", fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(origin);
  await page.waitForFunction(
    () =>
      document.querySelector("astro-island") &&
      !document.querySelector("astro-island").hasAttribute("ssr"),
  );
  assert.equal(await page.locator(".pin-spacer").count(), 0);
  assert.equal(await page.locator(".code-version:visible").count(), 2);
  await page.getByRole("button", { name: "03 Build" }).click();
  await expectPhase("synced");
  assert.equal(await page.locator('[data-code="resolver-after"]').isVisible(), true);
  await page.getByRole("button", { name: "01 Define" }).click();
  await expectPhase("initial");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.waitForSelector(".pin-spacer");
  assert.equal(
    await page.locator(".pin-spacer").count(),
    1,
    "Changing motion preferences should rebuild one timeline.",
  );

  await page.goto(`${origin}/docs/`);
  assert.equal(await page.locator("h1").innerText(), "Introduction");
  await page.goto(`${origin}/docs/tutorial/`);
  assert.equal(await page.locator("h1").innerText(), "Tutorial");
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
