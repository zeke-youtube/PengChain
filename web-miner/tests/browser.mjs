import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync, mkdtempSync, rmSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { createServer } from "node:http";
import { once } from "node:events";
import { openDb } from "../../server/dist/db.js";
import { buildApp } from "../../server/dist/app.js";
import { contentSecurityPolicy } from "../csp.mjs";

const require = createRequire(
  new URL("../../apps/pengminer/package.json", import.meta.url),
);
const { chromium } = require("playwright");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const temp = mkdtempSync(join(tmpdir(), "peng-web-miner-"));
const db = openDb(join(temp, "test.sqlite"));
db.prepare("UPDATE tokens SET difficulty=1 WHERE symbol='PENG'").run();
const app = buildApp(db);
let jobs = 0,
  submissions = [],
  faults = {},
  nonceReceipts = [];
const server = createServer(async (req, res) => {
  if (req.url === "/web-miner/") {
    res.writeHead(200, {
      "Content-Type": "text/html; charset=utf-8",
      "Content-Security-Policy": contentSecurityPolicy(html),
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
    });
    res.end(html);
    return;
  }
  try {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    let payload = Buffer.concat(chunks).toString();
    if (req.url.startsWith("/blockchain/api/mining/job")) {
      jobs++;
      if (faults.job503) {
        faults.job503--;
        res.writeHead(503, { "Content-Type": "application/json" });
        res.end('{"error":"temporary test outage"}');
        return;
      }
    }
    if (req.url === "/blockchain/api/mining/submit") {
      submissions.push(JSON.parse(payload));
      if (faults.reject) {
        faults.reject--;
        res.writeHead(400, { "Content-Type": "application/json" });
        res.end('{"error":"stale or already used job"}');
        return;
      }
      if (faults.invalid) {
        faults.invalid--;
        res.writeHead(400, { "Content-Type": "application/json" });
        res.end('{"error":"invalid solution"}');
        return;
      }
    }
    const result = await app.inject({
      method: req.method,
      url: req.url,
      headers: { ...req.headers, host: "localhost" },
      ...(payload ? { payload } : {}),
    });
    let body = result.body;
    if (
      req.url.startsWith("/blockchain/api/mining/job") &&
      result.statusCode === 200 &&
      faults.expire
    ) {
      faults.expire--;
      const job = JSON.parse(body);
      // Exercise actual server expiry, with the browser deadline already elapsed.
      db.prepare("UPDATE mining_jobs SET expires_at=? WHERE id=?").run(
        Date.now() - 1,
        job.jobId,
      );
      job.expiresAt = Date.now() - 1;
      job.issuedAt = job.expiresAt - 300000;
      body = JSON.stringify(job);
    }
    if (
      req.url === "/blockchain/api/mining/submit" &&
      result.statusCode === 200
    ) {
      nonceReceipts.push({
        submission: JSON.parse(payload),
        receipt: JSON.parse(body),
      });
      if (faults.dropReceipt) {
        faults.dropReceipt--;
        res.writeHead(502, { "Content-Type": "application/json" });
        res.end('{"error":"proxy lost acceptance response"}');
        return;
      }
    }
    const headers = { ...result.headers };
    delete headers["content-length"];
    res.writeHead(result.statusCode, headers);
    res.end(body);
  } catch (error) {
    res.writeHead(500);
    res.end(String(error));
  }
});
let browser;
const savedErrors = [];
try {
  await app.ready();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const origin = `http://127.0.0.1:${server.address().port}`;
  const created = await app.inject({
    method: "POST",
    url: "/blockchain/api/wallet/create",
    payload: { password: "web-miner-test-password" },
  });
  assert.equal(created.statusCode, 200);
  const wallet = created.json().walletId;
  browser = await chromium.launch({
    ...(process.env.CHROMIUM_PATH
      ? { executablePath: process.env.CHROMIUM_PATH }
      : {}),
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  const context = await browser.newContext({
    viewport: { width: 1360, height: 960 },
  });
  const page = await context.newPage();
  page.on("pageerror", (error) => savedErrors.push(error.message));
  page.on("console", (message) => {
    if (
      /violates.*Content Security Policy|Refused to execute|Refused to create/i.test(
        message.text(),
      )
    )
      savedErrors.push(message.text());
  });
  const waitLog = (text) =>
    page.locator("#log").getByText(text, { exact: false }).first().waitFor();
  const start = () =>
    page.getByRole("button", { name: "Start Mining" }).click();
  const stop = async () => {
    if (await page.locator("#stop").isEnabled())
      await page.locator("#stop").click();
    await page.waitForFunction(
      () => document.querySelector("#mining-state").textContent === "Idle",
    );
  };
  await page.goto(origin + "/web-miner/");
  await page.waitForFunction(
    () =>
      document.querySelector("#connection-text").textContent ===
      "Sign-in required",
  );
  assert.equal(jobs, 0);
  assert.equal(submissions.length, 0);
  assert.equal(await page.locator("#start").isDisabled(), true);
  await page.getByLabel("Wallet ID", { exact: true }).fill(wallet);
  await page
    .getByLabel("Password", { exact: true })
    .fill("web-miner-test-password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector("#start").disabled);
  assert.equal(await page.locator("#password").inputValue(), "");
  assert.equal(await page.locator("#wallet").innerText(), wallet);
  const cookie = (await context.cookies()).find(
    (cookie) => cookie.name === "peng_session",
  );
  assert.equal(cookie.httpOnly, true);
  assert.equal(cookie.sameSite, "Strict");
  assert.equal(await page.evaluate(() => localStorage.length), 0);
  assert.equal(jobs, 0, "Signing in must not issue jobs or start mining");
  const vector = await page.evaluate(async () => {
    const source = document.querySelector("#mining-core").textContent;
    const url = URL.createObjectURL(
      new Blob([source, "\n(", Pow.workerMain.toString(), ")();"], {
        type: "text/javascript",
      }),
    );
    const worker = new Worker(url);
    try {
      return await new Promise((resolve, reject) => {
        const timer = setTimeout(
          () => reject(new Error("Vector worker timeout")),
          10000,
        );
        worker.onerror = (event) => {
          clearTimeout(timer);
          reject(new Error(event.message));
        };
        worker.onmessage = (event) => {
          if (event.data.type !== "progress") {
            clearTimeout(timer);
            resolve(event.data);
          }
        };
        worker.postMessage({
          type: "start",
          job: {
            jobId: "job",
            tokenId: "PENG",
            walletId: "wallet",
            challenge: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            difficulty: 3,
          },
          network: "pengchain-v1",
          lane: 6306,
          count: 6307,
          deadline: Date.now() + 10000,
        });
      });
    } finally {
      worker.terminate();
      URL.revokeObjectURL(url);
    }
  });
  assert.equal(vector.type, "found");
  assert.equal(vector.nonce, "6306");
  assert.equal(vector.hashes, "1");
  console.log(
    "PASS: real browser Blob worker agrees with the Rust PengWallet vector",
  );
  const workerCount = await page.locator("#workers").getAttribute("max");
  if (Number(workerCount) > 1) await page.locator("#workers").fill("2");
  await start();
  await waitLog("Accepted by server:");
  await stop();
  assert.ok(
    Number((await page.locator("#hashes").innerText()).replaceAll(",", "")) > 0,
  );
  const confirmed = nonceReceipts.reduce(
    (sum, entry) => sum + entry.receipt.reward,
    0,
  );
  assert.equal(
    Number((await page.locator("#earned").innerText()).replaceAll(",", "")),
    confirmed,
  );
  assert.equal(
    db
      .prepare("SELECT amount FROM balances WHERE wallet_id=? AND token_id=?")
      .get(wallet, "PENG").amount,
    confirmed,
  );
  assert.equal(await page.locator("#active-workers").innerText(), "0");
  const submissionCount = submissions.length;
  await page.waitForTimeout(700);
  assert.equal(submissions.length, submissionCount);
  const replay = await page.evaluate(
    async (submission) =>
      (
        await fetch("/blockchain/api/mining/submit", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(submission),
        })
      ).status,
    submissions[0],
  );
  assert.equal(replay, 400);
  console.log(
    "PASS: cookie login; no automatic mining; Blob workers; real server-verified nonce/reward/balance; duplicate rejection; stop cleanup",
  );

  faults = { job503: 1, expire: 1, reject: 1 };
  const oldJobs = jobs,
    oldSubmissions = submissions.length;
  await start();
  await waitLog("temporary test outage");
  await waitLog("Job expired or nonce lanes completed");
  await waitLog("Submission rejected: stale");
  await page.waitForFunction(
    () => Number(document.querySelector("#accepted").textContent) >= 1,
  );
  await stop();
  assert.ok(jobs >= oldJobs + 4);
  assert.ok(submissions.length >= oldSubmissions + 2);
  assert.equal(await page.locator("#rejected").innerText(), "1");
  console.log(
    "PASS: temporary HTTP failure backoff, expired job refresh, stale rejection, continuous recovery",
  );

  faults = { invalid: 1 };
  await start();
  await waitLog("Check the network ID");
  await page.waitForFunction(
    () => document.querySelector("#mining-state").textContent === "Idle",
  );
  assert.equal(await page.locator("#earned").innerText(), "0");
  assert.equal(await page.locator("#rejected").innerText(), "1");
  console.log(
    "PASS: invalid proof response stops safely without crediting rewards",
  );

  faults = { dropReceipt: 1 };
  await start();
  await waitLog("Submission not confirmed");
  await stop();
  assert.equal(await page.locator("#earned").innerText(), "0");
  console.log(
    "PASS: lost acceptance response never creates locally confirmed earnings",
  );

  faults = {};
  db.prepare("UPDATE tokens SET difficulty=64 WHERE symbol='PENG'").run();
  await start();
  await page.waitForFunction(
    () => Number(document.querySelector("#active-workers").textContent) > 0,
  );
  await context.setOffline(true);
  await page.waitForFunction(
    () =>
      document.querySelector("#mining-state").textContent === "Reconnecting",
  );
  await stop();
  await context.setOffline(false);
  assert.equal(await page.locator("#active-workers").innerText(), "0");
  console.log("PASS: offline cancellation and Stop during backoff");

  await start();
  await page.waitForFunction(
    () => Number(document.querySelector("#active-workers").textContent) > 0,
  );
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      value: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(
    () => document.querySelector("#mining-state").textContent === "Idle",
  );
  assert.equal(await page.locator("#active-workers").innerText(), "0");
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      value: false,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  assert.equal(await page.locator("#stop").isDisabled(), true);
  console.log("PASS: hidden-tab cancellation never automatically resumes");

  const beforeSleepJobs = jobs;
  await start();
  await page.waitForFunction(
    () => Number(document.querySelector("#active-workers").textContent) > 0,
  );
  await page.evaluate(() => {
    // A second direct handler call must not create another in-page mining run.
    document.querySelector("#start").onclick();
    window.originalTestDateNow = Date.now;
    Date.now = () => window.originalTestDateNow() + 20000;
  });
  await page.waitForFunction(
    () => document.querySelector("#mining-state").textContent === "Idle",
  );
  await page.evaluate(() => {
    Date.now = window.originalTestDateNow;
    delete window.originalTestDateNow;
  });
  assert.equal(jobs, beforeSleepJobs + 1);
  assert.equal(await page.locator("#active-workers").innerText(), "0");
  console.log("PASS: sleep/clock-gap cancellation and single-run guard");

  await page.reload();
  await page.waitForFunction(() => !document.querySelector("#start").disabled);
  assert.equal(await page.locator("#earned").innerText(), "0");
  assert.equal(
    await page.locator("#wallet").innerText(),
    "Identified when you start mining",
  );
  assert.equal(await page.locator("#active-workers").innerText(), "0");
  await page.setViewportSize({ width: 320, height: 720 });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  if (process.env.SCREENSHOT_DIR) {
    mkdirSync(process.env.SCREENSHOT_DIR, { recursive: true });
    await page.screenshot({
      path: resolve(process.env.SCREENSHOT_DIR, "web-miner-mobile.png"),
      fullPage: true,
    });
    await page.setViewportSize({ width: 1360, height: 960 });
    await page.screenshot({
      path: resolve(process.env.SCREENSHOT_DIR, "web-miner-desktop.png"),
      fullPage: true,
    });
  }
  db.prepare("DELETE FROM sessions").run();
  await start();
  await waitLog("Session expired or revoked");
  await page.waitForFunction(
    () => document.querySelector("#mining-state").textContent === "Idle",
  );
  assert.equal(await page.locator("#start").isDisabled(), true);
  assert.equal(await page.locator("#balance").innerText(), "—");
  assert.equal(
    await page.locator("#connection-text").innerText(),
    "Sign-in required",
  );
  assert.deepEqual(savedErrors, []);
  console.log(
    "PASS: reload restores cookie without mining; mobile layout; revoked session; strict hash-based CSP",
  );
} finally {
  await browser?.close();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
  await app.close();
  db.close();
  rmSync(temp, { recursive: true, force: true });
}
