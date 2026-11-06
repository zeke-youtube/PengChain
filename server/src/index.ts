import { openDb } from "./db.js";
import { buildApp } from "./app.js";
const db = openDb(process.env.PENGCHAIN_DB ?? "pengchain.sqlite");
const app = buildApp(db);
const port = Number(process.env.PORT ?? 3000);
app.listen({ port, host: process.env.HOST ?? "0.0.0.0" }).catch((e) => {
  app.log.error(e);
  process.exit(1);
});
let closing = false;
const close = async () => {
  if (closing) return;
  closing = true;
  try {
    await app.close();
  } finally {
    db.close();
  }
  process.exitCode = 0;
};
process.once("SIGINT", () => void close());
process.once("SIGTERM", () => void close());
