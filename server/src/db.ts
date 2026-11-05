import Database from "better-sqlite3";
import { createHash, randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const root = dirname(fileURLToPath(import.meta.url));
export function openDb(path: string) {
  const db = new Database(path);
  db.pragma("journal_mode = WAL");
  db.pragma("foreign_keys = ON");
  db.exec(readFileSync(join(root, "../migrations/001_initial.sql"), "utf8"));
  const now = Date.now();
  const peng = db
    .prepare("SELECT id FROM tokens WHERE symbol=?")
    .get("PENG") as { id: string } | undefined;
  if (!peng)
    db.prepare(
      "INSERT INTO tokens(id,name,symbol,creator_wallet_id,max_supply,mineable,difficulty,created_at) VALUES(?,?,?,?,?,?,?,?)",
    ).run("PENG", "PengChain", "PENG", null, null, 1, 3, now);
  return db;
}
export function audit(
  db: Database.Database,
  eventType: string,
  payload: unknown,
) {
  const prev =
    (
      db
        .prepare("SELECT hash FROM audit_log ORDER BY id DESC LIMIT 1")
        .get() as { hash: string } | undefined
    )?.hash ?? "";
  const createdAt = Date.now();
  const body = JSON.stringify(payload);
  const hash = createHash("sha256")
    .update(prev + eventType + body + createdAt)
    .digest("hex");
  db.prepare(
    "INSERT INTO audit_log(event_type,payload,created_at,previous_hash,hash) VALUES(?,?,?,?,?)",
  ).run(eventType, body, createdAt, prev, hash);
}
export const id = () => randomBytes(16).toString("hex");
