import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export function contentSecurityPolicy(html) {
  const hash = (text) =>
    `'sha256-${createHash("sha256").update(text).digest("base64")}'`;
  const scripts = [...html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)].map(
    (match) => hash(match[1]),
  );
  const styles = [...html.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(
    (match) => hash(match[1]),
  );
  return `default-src 'none'; script-src ${scripts.join(" ")}; style-src ${styles.join(" ")}; connect-src 'self'; worker-src blob:; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'`;
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  console.log(
    `add_header Content-Security-Policy "${contentSecurityPolicy(html)}" always;`,
  );
}
