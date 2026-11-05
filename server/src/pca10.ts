import { createHash } from "node:crypto";
export function pca10(input: Buffer): Buffer {
  let d = input;
  for (let i = 0; i < 10; i++) d = createHash("sha256").update(d).digest();
  return d;
}
export function meetsDifficulty(digest: Buffer, difficulty: number): boolean {
  return digest.toString("hex").startsWith("0".repeat(difficulty));
}
export function canonicalMiningInput(
  version: number,
  network: string,
  jobId: string,
  tokenId: string,
  walletId: string,
  challenge: Buffer,
  nonce: bigint,
): Buffer {
  const enc = (s: string) => Buffer.from(s, "utf8");
  const fields = [
    Buffer.from([version]),
    enc(network),
    enc(jobId),
    enc(tokenId),
    enc(walletId),
    challenge,
  ];
  const n = Buffer.alloc(8);
  n.writeBigUInt64BE(nonce);
  return Buffer.concat(
    fields
      .map((x) => {
        const l = Buffer.alloc(4);
        l.writeUInt32BE(x.length);
        return Buffer.concat([l, x]);
      })
      .concat(n),
  );
}
