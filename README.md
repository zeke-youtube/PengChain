# PengChain V1

PengChain is a centralized proof-of-work ledger by PikaStudio. The server is authoritative and persists wallet accounts, sessions, tokens, balances, transfers, mining jobs, rewards, and tamper-evident audit records in SQLite WAL mode.

Run the server with `cd server`, `npm install`, and `npm run dev`. Set `PENGCHAIN_DB`, `PORT`, `HOST`, and `NETWORK_ID` through the environment. Docker deployment is defined in `server/compose.yaml` and persists the database in a named volume.

The Rust CLI is intentionally not included until the server acceptance tests pass. Production deployment should put HTTPS in front of the service, restrict the local first-wallet bootstrap channel at the network layer, and maintain external database backups or audit checkpoints.
