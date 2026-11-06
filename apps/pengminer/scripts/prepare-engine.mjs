// Build our own CLI from source. Do not download or execute a third-party engine.
import { spawnSync } from "node:child_process";
import { mkdirSync, copyFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL("../../../", import.meta.url));
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
const host = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
if (host.status !== 0)
  throw new Error("Install Rust and put cargo/rustc on PATH.");
const native = host.stdout.match(/^host: (.+)$/m)?.[1];
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ?? process.env.PENGMINER_TARGET ?? native;
if (!triple || !/^[A-Za-z0-9_-]+$/.test(triple))
  throw new Error("Invalid Rust target triple.");
run("cargo", [
  "build",
  "--release",
  "--locked",
  "--manifest-path",
  "client/Cargo.toml",
  ...(triple === native ? [] : ["--target", triple]),
]);
const extension = triple.includes("windows") ? ".exe" : "";
const source = path.join(
  root,
  "client",
  "target",
  ...(triple === native ? [] : [triple]),
  "release",
  `pengwallet${extension}`,
);
const sidecars = path.join(root, "apps", "pengminer", "src-tauri", "binaries");
mkdirSync(sidecars, { recursive: true });
copyFileSync(source, path.join(sidecars, `pengwallet-${triple}${extension}`));
// No-bundle builds/dev runs also launch an adjacent, separate executable.
for (const profile of ["debug", "release"]) {
  for (const target of [
    path.join(root, "apps", "pengminer", "src-tauri", "target", profile),
    path.join(
      root,
      "apps",
      "pengminer",
      "src-tauri",
      "target",
      triple,
      profile,
    ),
  ]) {
    mkdirSync(target, { recursive: true });
    copyFileSync(source, path.join(target, `pengwallet${extension}`));
  }
}
console.log(`Prepared separate PengWallet engine (${triple}).`);
