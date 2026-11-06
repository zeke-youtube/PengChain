import Fastify, { FastifyInstance } from "fastify";
import cookie from "@fastify/cookie";
import helmet from "@fastify/helmet";
import rateLimit from "@fastify/rate-limit";
import argon2 from "argon2";
import { randomBytes } from "node:crypto";
import { z } from "zod";
import Database from "better-sqlite3";
import { audit, id } from "./db.js";
import { canonicalMiningInput, meetsDifficulty, pca10 } from "./pca10.js";
import { ADJUSTMENT_WINDOW, adjustDifficulty } from "./difficulty.js";
const now = () => Date.now();
const walletSchema = z.object({ password: z.string().min(8).max(256) });
const tokenSchema = z.object({
  name: z.string().min(1).max(80),
  symbol: z.string().regex(/^[A-Za-z][A-Za-z0-9]{0,15}$/),
  maxSupply: z.number().int().positive().nullable().optional(),
  mineable: z.boolean(),
  miningReward: z.number().int().positive().optional(),
  difficulty: z.number().int().min(1).max(64).optional(),
  initialIssuance: z.number().int().nonnegative().optional(),
});
export function buildApp(db: Database.Database): FastifyInstance {
  const app = Fastify({ logger: false });
  app.register(cookie);
  app.register(helmet);
  app.register(rateLimit, { max: 100, timeWindow: "1 minute" });
  const auth = async (req: any, reply: any) => {
    const sid = req.cookies.peng_session;
    const s =
      sid &&
      (db
        .prepare("SELECT * FROM sessions WHERE id=? AND expires_at>?")
        .get(sid, now()) as any);
    if (!s) return reply.code(401).send({ error: "authentication required" });
    const w = db
      .prepare("SELECT id,role,banned_until FROM wallets WHERE id=?")
      .get(s.wallet_id) as any;
    if (!w) return reply.code(401).send({ error: "authentication required" });
    req.wallet = w;
  };
  const json = (x: any) => {
    const p = Buffer.from(JSON.stringify(x));
    return p.toString("base64url");
  };
  app.get("/health", async () => ({ ok: true }));
  app.post("/blockchain/api/wallet/create", async (req: any, reply) => {
    const p = walletSchema.safeParse(req.body);
    if (!p.success) return reply.code(400).send({ error: "invalid request" });
    const hash = await argon2.hash(p.data.password, { type: argon2.argon2id });
    const wid = "PENG-" + randomBytes(4).toString("hex").toUpperCase();
    try {
      db.transaction(() => {
        const owner = !db.prepare("SELECT 1 FROM wallets LIMIT 1").get();
        db.prepare(
          "INSERT INTO wallets(id,password_hash,created_at,role) VALUES(?,?,?,?)",
        ).run(wid, hash, now(), owner ? "OWNER" : "USER");
        audit(db, "wallet.created", { walletId: wid, owner });
      })();
    } catch {
      return reply.code(409).send({ error: "wallet creation conflict" });
    }
    return {
      walletId: wid,
      role: (db.prepare("SELECT role FROM wallets WHERE id=?").get(wid) as any)
        .role,
    };
  });
  app.post("/blockchain/api/wallet/login", async (req: any, reply) => {
    const p = walletSchema.extend({ walletId: z.string() }).safeParse(req.body);
    if (!p.success) return reply.code(400).send({ error: "invalid request" });
    const w = db
      .prepare("SELECT * FROM wallets WHERE id=?")
      .get(p.data.walletId) as any;
    if (!w || !(await argon2.verify(w.password_hash, p.data.password)))
      return reply.code(401).send({ error: "invalid credentials" });
    const sid = id();
    db.prepare("INSERT INTO sessions VALUES(?,?,?,?)").run(
      sid,
      w.id,
      now() + 86400000,
      now(),
    );
    audit(db, "wallet.login", { walletId: w.id });
    reply.setCookie("peng_session", sid, {
      httpOnly: true,
      sameSite: "strict",
      secure: process.env.NODE_ENV === "production",
      path: "/",
      maxAge: 86400,
    });
    return { walletId: w.id, role: w.role };
  });
  app.post(
    "/blockchain/api/wallet/logout",
    { preHandler: auth },
    async (req: any, reply) => {
      audit(db, "wallet.logout", { walletId: req.wallet.id });
      db.prepare("DELETE FROM sessions WHERE id=?").run(
        req.cookies.peng_session,
      );
      reply.clearCookie("peng_session", { path: "/" });
      return { ok: true };
    },
  );
  app.get(
    "/blockchain/api/wallet/balance",
    { preHandler: auth },
    async (req: any) =>
      db
        .prepare(
          "SELECT b.token_id tokenId,t.symbol,b.amount FROM balances b JOIN tokens t ON t.id=b.token_id WHERE b.wallet_id=?",
        )
        .all(req.wallet.id),
  );
  app.post(
    "/blockchain/api/token",
    { preHandler: auth },
    async (req: any, reply) => {
      if (req.wallet.banned_until && req.wallet.banned_until > now())
        return reply.code(403).send({ error: "wallet banned" });
      const p = tokenSchema.safeParse(req.body);
      if (!p.success) return reply.code(400).send({ error: "invalid request" });
      const d = p.data;
      const symbol = d.symbol.toUpperCase();
      if (symbol === "PENG")
        return reply.code(409).send({ error: "PENG is reserved" });
      const tokenId = "TOK-" + randomBytes(10).toString("hex");
      const max = d.maxSupply ?? null;
      const issue = d.mineable ? 0 : (d.initialIssuance ?? 0);
      if (max !== null && issue > max)
        return reply
          .code(400)
          .send({ error: "initial issuance exceeds supply" });
      try {
        db.transaction(() => {
          db.prepare("INSERT INTO tokens VALUES(?,?,?,?,?,?,?,?,?)").run(
            tokenId,
            d.name,
            symbol,
            req.wallet.id,
            max,
            issue,
            d.mineable ? 1 : 0,
            d.difficulty ?? 3,
            now(),
          );
          if (issue)
            db.prepare("INSERT INTO balances VALUES(?,?,?)").run(
              req.wallet.id,
              tokenId,
              issue,
            );
          audit(db, "token.created", {
            tokenId,
            symbol,
            creatorWalletId: req.wallet.id,
          });
        })();
      } catch {
        return reply.code(409).send({ error: "token symbol already exists" });
      }
      return { tokenId, symbol };
    },
  );
  const tokenSelect =
    "SELECT id,name,symbol,creator_wallet_id creatorWalletId,max_supply maxSupply,issued_supply issuedSupply,mineable,difficulty,created_at createdAt FROM tokens";
  app.get("/blockchain/api/token", async (req: any) => {
    const q = z
      .object({
        page: z.coerce.number().int().min(1).default(1),
        limit: z.coerce.number().int().min(1).max(100).default(25),
      })
      .parse(req.query);
    return db
      .prepare(tokenSelect + " ORDER BY created_at DESC LIMIT ? OFFSET ?")
      .all(q.limit, (q.page - 1) * q.limit);
  });
  app.get("/blockchain/api/token/search", async (req: any) => {
    const q = z
      .object({
        q: z.string().min(1),
        page: z.coerce.number().int().min(1).default(1),
        limit: z.coerce.number().int().min(1).max(100).default(25),
      })
      .parse(req.query);
    const like = "%" + q.q + "%";
    return db
      .prepare(
        tokenSelect +
          " WHERE id=? OR name LIKE ? OR symbol LIKE ? COLLATE NOCASE ORDER BY created_at DESC LIMIT ? OFFSET ?",
      )
      .all(q.q, like, like, q.limit, (q.page - 1) * q.limit);
  });
  app.get("/blockchain/api/token/:id", async (req: any, reply) => {
    const t = db.prepare(tokenSelect + " WHERE id=?").get(req.params.id);
    return t ?? reply.code(404).send({ error: "token not found" });
  });
  app.post(
    "/blockchain/api/transfer",
    { preHandler: auth },
    async (req: any, reply) => {
      if (req.wallet.banned_until && req.wallet.banned_until > now())
        return reply.code(403).send({ error: "wallet banned" });
      const p = z
        .object({
          recipientWalletId: z.string(),
          tokenId: z.string(),
          amount: z.number().int().positive(),
          idempotencyKey: z.string().min(8).max(128),
        })
        .safeParse(req.body);
      if (!p.success) return reply.code(400).send({ error: "invalid request" });
      const d = p.data;
      try {
        const result = db.transaction(() => {
          const existing = db
            .prepare("SELECT * FROM transfers WHERE idempotency_key=?")
            .get(d.idempotencyKey);
          if (existing) return existing;
          const sender = db
            .prepare(
              "SELECT amount FROM balances WHERE wallet_id=? AND token_id=?",
            )
            .get(req.wallet.id, d.tokenId) as any;
          if (!sender || sender.amount < d.amount)
            throw new Error("insufficient balance");
          if (
            !db
              .prepare("SELECT 1 FROM wallets WHERE id=?")
              .get(d.recipientWalletId)
          )
            throw new Error("recipient not found");
          db.prepare(
            "INSERT INTO balances VALUES(?,?,?) ON CONFLICT(wallet_id,token_id) DO UPDATE SET amount=amount+excluded.amount",
          ).run(d.recipientWalletId, d.tokenId, d.amount);
          db.prepare(
            "UPDATE balances SET amount=amount-? WHERE wallet_id=? AND token_id=?",
          ).run(d.amount, req.wallet.id, d.tokenId);
          const tx = {
            id: id(),
            ...d,
            senderWalletId: req.wallet.id,
            createdAt: now(),
          };
          db.prepare("INSERT INTO transfers VALUES(?,?,?,?,?,?,?)").run(
            tx.id,
            d.idempotencyKey,
            req.wallet.id,
            d.recipientWalletId,
            d.tokenId,
            d.amount,
            tx.createdAt,
          );
          audit(db, "transfer.accepted", tx);
          return tx;
        })();
        return result;
      } catch (e: any) {
        return reply.code(400).send({
          error:
            e.message === "insufficient balance"
              ? "insufficient balance"
              : "transfer rejected",
        });
      }
    },
  );
  app.get(
    "/blockchain/api/transactions",
    { preHandler: auth },
    async (req: any) =>
      db
        .prepare(
          "SELECT * FROM transfers WHERE sender_wallet_id=? OR recipient_wallet_id=? ORDER BY created_at DESC LIMIT 100",
        )
        .all(req.wallet.id, req.wallet.id),
  );
  app.get(
    "/blockchain/api/mining/job",
    { preHandler: auth },
    async (req: any, reply) => {
      const symbol = String((req.query as any)?.symbol ?? "PENG").toUpperCase();
      const t = db
        .prepare("SELECT * FROM tokens WHERE symbol=?")
        .get(symbol) as any;
      if (!t || !t.mineable)
        return reply.code(400).send({ error: "mineable token not found" });
      const jobId = id(),
        challenge = randomBytes(32),
        issued = now(),
        expires = issued + 300000;
      db.prepare("INSERT INTO mining_jobs VALUES(?,?,?,?,?,?,?,?,?,?)").run(
        jobId,
        t.id,
        req.wallet.id,
        challenge,
        issued,
        expires,
        t.difficulty,
        t.symbol === "PENG" ? 10 : 1,
        null,
        null,
      );
      audit(db, "mining.job_issued", {
        jobId,
        tokenId: t.id,
        walletId: req.wallet.id,
        difficulty: t.difficulty,
        expiresAt: expires,
      });
      return {
        jobId,
        tokenId: t.id,
        symbol: t.symbol,
        walletId: req.wallet.id,
        challenge: challenge.toString("base64url"),
        difficulty: t.difficulty,
        issuedAt: issued,
        expiresAt: expires,
        reward: t.symbol === "PENG" ? 10 : 1,
      };
    },
  );
  app.post(
    "/blockchain/api/mining/submit",
    { preHandler: auth },
    async (req: any, reply) => {
      const p = z
        .object({ jobId: z.string(), nonce: z.string().regex(/^\d+$/) })
        .safeParse(req.body);
      if (!p.success) return reply.code(400).send({ error: "invalid request" });
      try {
        const result = db.transaction(() => {
          const j = db
            .prepare(
              "SELECT j.*,t.max_supply,t.issued_supply,t.symbol FROM mining_jobs j JOIN tokens t ON t.id=j.token_id WHERE j.id=?",
            )
            .get(p.data.jobId) as any;
          if (
            !j ||
            j.wallet_id !== req.wallet.id ||
            j.accepted_at ||
            j.expires_at < now()
          )
            throw new Error("stale or already used job");
          const digest = pca10(
            canonicalMiningInput(
              1,
              process.env.NETWORK_ID ?? "pengchain-v1",
              j.id,
              j.token_id,
              j.wallet_id,
              j.challenge,
              BigInt(p.data.nonce),
            ),
          );
          if (!meetsDifficulty(digest, j.difficulty))
            throw new Error("invalid solution");
          if (
            j.max_supply !== null &&
            j.issued_supply + j.reward > j.max_supply
          )
            throw new Error("supply exhausted");
          const accepted = now();
          db.prepare(
            "UPDATE mining_jobs SET accepted_at=?,nonce=? WHERE id=? AND accepted_at IS NULL",
          ).run(accepted, p.data.nonce, j.id);
          db.prepare("INSERT INTO mining_solutions VALUES(?,?,?,?,?,?,?)").run(
            id(),
            j.id,
            j.token_id,
            j.wallet_id,
            p.data.nonce,
            digest.toString("hex"),
            accepted,
          );
          db.prepare(
            "UPDATE tokens SET issued_supply=issued_supply+? WHERE id=?",
          ).run(j.reward, j.token_id);
          db.prepare(
            "INSERT INTO balances VALUES(?,?,?) ON CONFLICT(wallet_id,token_id) DO UPDATE SET amount=amount+excluded.amount",
          ).run(j.wallet_id, j.token_id, j.reward);
          const acceptedCount = (
            db
              .prepare(
                "SELECT COUNT(*) count FROM mining_solutions WHERE token_id=?",
              )
              .get(j.token_id) as { count: number }
          ).count;
          let newDifficulty = j.difficulty;
          if (acceptedCount % ADJUSTMENT_WINDOW === 0) {
            const timestamps = (
              db
                .prepare(
                  "SELECT accepted_at acceptedAt FROM mining_solutions WHERE token_id=? ORDER BY accepted_at DESC LIMIT ?",
                )
                .all(j.token_id, ADJUSTMENT_WINDOW) as Array<{
                acceptedAt: number;
              }>
            )
              .map((row) => row.acceptedAt)
              .reverse();
            const adjustment = adjustDifficulty(j.difficulty, timestamps);
            newDifficulty = adjustment.difficulty;
            db.prepare("UPDATE tokens SET difficulty=? WHERE id=?").run(
              newDifficulty,
              j.token_id,
            );
            audit(db, "mining.difficulty_adjusted", {
              tokenId: j.token_id,
              previousDifficulty: j.difficulty,
              newDifficulty,
              acceptedCount,
              averageIntervalSeconds: adjustment.averageIntervalSeconds,
              direction: adjustment.direction,
            });
          }
          audit(db, "mining.accepted", {
            jobId: j.id,
            tokenId: j.token_id,
            walletId: j.wallet_id,
            reward: j.reward,
            digest: digest.toString("hex"),
          });
          return {
            accepted: true,
            reward: j.reward,
            digest: digest.toString("hex"),
            difficulty: newDifficulty,
          };
        })();
        return result;
      } catch (e: any) {
        return reply.code(400).send({ error: e.message });
      }
    },
  );
  app.get("/blockchain/api/mining/stats", async () =>
    db
      .prepare(
        "SELECT t.symbol,t.difficulty,COUNT(s.id) acceptedSolutions,COALESCE(SUM(j.reward),0) totalRewards FROM tokens t LEFT JOIN mining_jobs j ON j.token_id=t.id AND j.accepted_at IS NOT NULL LEFT JOIN mining_solutions s ON s.job_id=j.id GROUP BY t.id",
      )
      .all(),
  );
  app.get("/blockchain/api/logs", async (req: any) => {
    const q = z
      .object({
        type: z.string().optional(),
        page: z.coerce.number().int().min(1).default(1),
        limit: z.coerce.number().int().min(1).max(100).default(50),
      })
      .parse(req.query);
    return db
      .prepare(
        "SELECT id,event_type eventType,payload,created_at createdAt,hash FROM audit_log " +
          (q.type ? "WHERE event_type LIKE ? " : "") +
          "ORDER BY id DESC LIMIT ? OFFSET ?",
      )
      .all(
        ...(q.type ? ["%" + q.type + "%"] : []),
        q.limit,
        (q.page - 1) * q.limit,
      );
  });
  return app;
}
