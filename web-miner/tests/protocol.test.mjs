import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash, webcrypto } from "node:crypto";
import vm from "node:vm";
import {
  canonicalMiningInput,
  pca10,
  meetsDifficulty,
} from "../../server/dist/pca10.js";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const source = html.match(/<script id="mining-core">([\s\S]*?)<\/script>/)[1];
const context = vm.createContext({
  TextEncoder,
  Uint8Array,
  DataView,
  atob,
  crypto: webcrypto,
  performance,
  Date,
});
const Pow = vm.runInContext(source + "\nPow", context);
const job = {
  jobId: "job",
  tokenId: "PENG",
  walletId: "wallet",
  challenge: Buffer.alloc(32).toString("base64url"),
  difficulty: 3,
};
const bytes = (value) => Buffer.from(value);

test("canonical encoding matches the server for UTF-8 fields and all uint64 boundaries", () => {
  const unicode = { ...job, jobId: "job-🐧", walletId: "wallet-é" };
  for (const j of [job, unicode])
    for (const nonce of [
      0n,
      1n,
      0x0102030405060708n,
      1n << 63n,
      (1n << 64n) - 1n,
    ]) {
      const input = Pow.input(Pow.prefix(j, "pengchain-v1"), nonce);
      assert.deepEqual(
        bytes(input),
        canonicalMiningInput(
          1,
          "pengchain-v1",
          j.jobId,
          j.tokenId,
          j.walletId,
          Buffer.alloc(32),
          nonce,
        ),
      );
      assert.equal(
        bytes(input).subarray(-8).toString("hex"),
        nonce.toString(16).padStart(16, "0"),
      );
    }
  assert.throws(() => Pow.input(Pow.prefix(job, "pengchain-v1"), -1n));
  assert.throws(() => Pow.input(Pow.prefix(job, "pengchain-v1"), 1n << 64n));
  assert.throws(() =>
    Pow.prefix({ ...job, challenge: "invalid!" }, "pengchain-v1"),
  );
  assert.throws(() => Pow.prefix({ ...job, difficulty: 65 }, "pengchain-v1"));
});

test("ten rounds use raw digests and match Rust PengWallet raw_digest_vector", async () => {
  let rounds = 0;
  const digest = await Pow.hash(Buffer.from("pengchain"), (data) => {
    assert.equal(data.length, rounds ? 32 : 9);
    rounds++;
    return createHash("sha256").update(data).digest();
  });
  assert.equal(rounds, 10);
  assert.equal(
    bytes(digest).toString("hex"),
    "ab2b961b9fa292a39d4a71668ac6449304a60d680615471fd352ea111226bddb",
  );
  assert.deepEqual(bytes(digest), pca10(Buffer.from("pengchain")));
});

test("PengWallet canonical valid nonce 6306 and invalid nonce zero match server", async () => {
  const prefix = Pow.prefix(job, "pengchain-v1");
  for (const [nonce, valid] of [
    [6306n, true],
    [0n, false],
  ]) {
    const digest = await Pow.hash(Pow.input(prefix, nonce));
    assert.equal(Pow.meetsDifficulty(digest, 3), valid);
    assert.equal(meetsDifficulty(bytes(digest), 3), valid);
    assert.deepEqual(bytes(digest), pca10(bytes(Pow.input(prefix, nonce))));
  }
});

test("difficulty checks full bytes and odd hex nibbles without rounding", () => {
  const digest = new Uint8Array(32);
  digest[1] = 0x0f;
  assert.equal(Pow.meetsDifficulty(digest, 3), true);
  assert.equal(Pow.meetsDifficulty(digest, 4), false);
  assert.equal(Pow.meetsDifficulty(digest, 65), false);
  assert.equal(Pow.meetsDifficulty(new Uint8Array(32), 64), true);
});

test("worker nonce partitions never overlap and do not wrap uint64", () => {
  for (const count of [1, 2, 3, 8, 16]) {
    const seen = new Set();
    for (let lane = 0; lane < count; lane++)
      for (let index = 0n; index < 100n; index++) {
        const nonce = Pow.laneNonce(lane, count, index);
        assert.equal(seen.has(nonce), false);
        seen.add(nonce);
      }
    assert.equal(seen.size, count * 100);
    const maxIndex = Pow.MAX_NONCE / BigInt(count) + 1n;
    assert.equal(Pow.laneNonce(0, count, maxIndex), null);
  }
  assert.throws(() => Pow.laneNonce(2, 2, 0n));
});

test("expiry checks wall time and monotonic time", () => {
  assert.equal(Pow.expired(Date.now() - 1, Infinity), true);
  assert.equal(Pow.expired(Infinity, performance.now() - 1), true);
  assert.equal(
    Pow.expired(Date.now() + 60000, performance.now() + 60000),
    false,
  );
});
