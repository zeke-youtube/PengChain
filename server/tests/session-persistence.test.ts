import { describe, expect, it } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { openDb } from "../src/db.js";
import { buildApp } from "../src/app.js";

describe("Remembered native sessions", () => {
  it("survives a real database restart and rejects a revoked session", async () => {
    const directory = mkdtempSync(join(tmpdir(), "peng-session-"));
    const file = join(directory, "ledger.sqlite");
    let db = openDb(file);
    let app = buildApp(db);
    try {
      const created = await app.inject({
        method: "POST",
        url: "/blockchain/api/wallet/create",
        payload: { password: "remembered-password" },
      });
      const walletId = created.json().walletId;
      const login = await app.inject({
        method: "POST",
        url: "/blockchain/api/wallet/login",
        payload: {
          walletId,
          password: "remembered-password",
          rememberMe: true,
        },
      });
      expect(login.statusCode).toBe(200);
      const cookie = String(login.headers["set-cookie"]).split(";")[0];
      await app.close();
      db.close();
      db = openDb(file);
      app = buildApp(db);
      const balance = () =>
        app.inject({
          method: "GET",
          url: "/blockchain/api/wallet/balance",
          headers: { cookie },
        });
      expect((await balance()).statusCode).toBe(200);
      db.prepare("DELETE FROM sessions WHERE wallet_id=?").run(walletId);
      expect((await balance()).statusCode).toBe(401);
    } finally {
      await app.close();
      db.close();
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
