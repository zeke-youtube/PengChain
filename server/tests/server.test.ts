import { describe, expect, it, afterEach } from "vitest";
import Database from "better-sqlite3";
import { readFileSync } from "node:fs";
import { buildApp } from "../src/app.js";
import { canonicalMiningInput, meetsDifficulty, pca10 } from "../src/pca10.js";
const resources: any[] = [];
afterEach(async () => {
  for (const x of resources.splice(0)) {
    await x.app.close();
    x.db.close();
  }
});
function make() {
  const db = new Database(":memory:");
  db.exec("PRAGMA foreign_keys=ON");
  db.exec(
    readFileSync(
      new URL("../migrations/001_initial.sql", import.meta.url),
      "utf8",
    ),
  );
  db.prepare(
    "INSERT INTO tokens(id,name,symbol,creator_wallet_id,max_supply,issued_supply,mineable,difficulty,created_at) VALUES(?,?,?,?,?,?,?,?,?)",
  ).run("PENG", "PengChain", "PENG", null, null, 0, 1, 1, Date.now());
  const app = buildApp(db);
  resources.push({ app, db });
  return { app, db };
}
async function create(app: any, password = "password123") {
  const r = await app.inject({
    method: "POST",
    url: "/blockchain/api/wallet/create",
    payload: { password },
  });
  return r.json().walletId;
}
async function login(app: any, walletId: string) {
  const r = await app.inject({
    method: "POST",
    url: "/blockchain/api/wallet/login",
    payload: { walletId, password: "password123" },
  });
  return r.headers["set-cookie"]?.split(";")[0] ?? "";
}
describe("PengChain server", () => {
  it("assigns exactly one owner under concurrent creation", async () => {
    const { app, db } = make();
    const ids = await Promise.all(Array.from({ length: 8 }, () => create(app)));
    expect(new Set(ids).size).toBe(8);
    expect(
      (
        db
          .prepare("SELECT COUNT(*) c FROM wallets WHERE role='OWNER'")
          .get() as any
      ).c,
    ).toBe(1);
  });
  it("authenticates and rejects invalid password", async () => {
    const { app } = make();
    const walletId = await create(app);
    const ok = await app.inject({
      method: "POST",
      url: "/blockchain/api/wallet/login",
      payload: { walletId, password: "password123" },
    });
    expect(ok.statusCode).toBe(200);
    const bad = await app.inject({
      method: "POST",
      url: "/blockchain/api/wallet/login",
      payload: { walletId, password: "wrong-password" },
    });
    expect(bad.statusCode).toBe(401);
  });
  it("keeps remembered native logins until sign-out or revocation", async () => {
    const { app, db } = make();
    const walletId = await create(app);
    const response = await app.inject({
      method: "POST",
      url: "/blockchain/api/wallet/login",
      payload: { walletId, password: "password123", rememberMe: true },
    });
    expect(response.statusCode).toBe(200);
    const cookie = String(response.headers["set-cookie"]).split(";")[0];
    const session = db
      .prepare("SELECT expires_at FROM sessions WHERE wallet_id=?")
      .get(walletId) as { expires_at: number };
    expect(session.expires_at).toBe(Number.MAX_SAFE_INTEGER);
    const balance = () =>
      app.inject({
        method: "GET",
        url: "/blockchain/api/wallet/balance",
        headers: { cookie },
      });
    expect((await balance()).statusCode).toBe(200);
    await app.inject({
      method: "POST",
      url: "/blockchain/api/wallet/logout",
      headers: { cookie },
    });
    expect((await balance()).statusCode).toBe(401);
    const regular = await login(app, walletId);
    db.prepare("UPDATE sessions SET expires_at=0 WHERE wallet_id=?").run(
      walletId,
    );
    expect(
      (
        await app.inject({
          method: "GET",
          url: "/blockchain/api/wallet/balance",
          headers: { cookie: regular },
        })
      ).statusCode,
    ).toBe(401);
  });
  it("rejects PENG recreation and symbol collision", async () => {
    const { app } = make();
    const walletId = await create(app);
    const cookie = await login(app, walletId);
    expect(
      (
        await app.inject({
          method: "POST",
          url: "/blockchain/api/token",
          headers: { cookie },
          payload: {
            name: "Nope",
            symbol: "PENG",
            mineable: false,
            initialIssuance: 0,
          },
        })
      ).statusCode,
    ).toBe(409);
    expect(
      (
        await app.inject({
          method: "POST",
          url: "/blockchain/api/token",
          headers: { cookie },
          payload: {
            name: "Fish",
            symbol: "FISH",
            mineable: false,
            maxSupply: 100,
            initialIssuance: 10,
          },
        })
      ).statusCode,
    ).toBe(200);
    expect(
      (
        await app.inject({
          method: "POST",
          url: "/blockchain/api/token",
          headers: { cookie },
          payload: {
            name: "Other",
            symbol: "fish",
            mineable: false,
            initialIssuance: 0,
          },
        })
      ).statusCode,
    ).toBe(409);
  });
  it("accepts a real decimal mining nonce and rewards a job only once", async () => {
    const { app, db } = make();
    const walletId = await create(app);
    const cookie = await login(app, walletId);
    const response = await app.inject({
      method: "GET",
      url: "/blockchain/api/mining/job?symbol=PENG",
      headers: { cookie },
    });
    expect(response.statusCode).toBe(200);
    const job = response.json();
    let nonce = 0n;
    for (; nonce < 100000n; nonce++) {
      const digest = pca10(
        canonicalMiningInput(
          1,
          process.env.NETWORK_ID ?? "pengchain-v1",
          job.jobId,
          job.tokenId,
          job.walletId,
          Buffer.from(job.challenge, "base64url"),
          nonce,
        ),
      );
      if (meetsDifficulty(digest, job.difficulty)) break;
    }
    expect(nonce).toBeLessThan(100000n);
    const submit = () =>
      app.inject({
        method: "POST",
        url: "/blockchain/api/mining/submit",
        headers: { cookie },
        payload: { jobId: job.jobId, nonce: nonce.toString() },
      });
    const accepted = await submit();
    expect(accepted.statusCode).toBe(200);
    const receipt = accepted.json();
    expect(receipt).toMatchObject({ accepted: true });
    expect(receipt.reward).toBeGreaterThanOrEqual(5);
    expect(receipt.reward).toBeLessThanOrEqual(15);
    const replay = await submit();
    expect(replay.statusCode).toBe(400);
    expect(replay.json()).toEqual({ error: "stale or already used job" });
    expect(
      db
        .prepare(
          "SELECT amount FROM balances WHERE wallet_id=? AND token_id='PENG'",
        )
        .get(walletId),
    ).toEqual({ amount: receipt.reward });
    expect(
      db.prepare("SELECT issued_supply FROM tokens WHERE id='PENG'").get(),
    ).toEqual({ issued_supply: receipt.reward });
    expect(
      db.prepare("SELECT COUNT(*) count FROM mining_solutions").get(),
    ).toEqual({ count: 1 });
  });
  it("PCA-10 is deterministic and uses raw digests", () => {
    const input = Buffer.from("pengchain");
    expect(pca10(input).toString("hex")).toBe(pca10(input).toString("hex"));
    expect(pca10(input)).toHaveLength(32);
  });
});
