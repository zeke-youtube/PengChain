import { openDb } from "./db.js";
import { buildApp } from "./app.js";
const db = openDb(process.env.PENGCHAIN_DB ?? "pengchain.sqlite");
const app = buildApp(db);
const port = Number(process.env.PORT ?? 3000);
app.listen({ port, host: process.env.HOST ?? "0.0.0.0" }).catch((e) => {
  app.log.error(e);
  process.exit(1);
});
const close = async () => {
  await app.close();
  db.close();
  process.exit(0);
};
process.on("SIGINT", close);
process.on("SIGTERM", close);
