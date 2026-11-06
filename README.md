# PengChain V1

PengChain is a centralized proof-of-work ledger by PikaStudio. The server is authoritative and persists wallet accounts, sessions, tokens, balances, transfers, mining jobs, rewards, and tamper-evident audit records in SQLite WAL mode.

Run the server with `cd server`, `npm install`, and `npm run dev`. Set `PENGCHAIN_DB`, `PORT`, `HOST`, and `NETWORK_ID` through the environment. Docker deployment is defined in `server/compose.yaml` and persists the database in a named volume.

The PengWallet Rust CLI lives under `client/`; see its [development instructions](client/README.md). The server acceptance matrix and other V1 work described in `AGENTS.md` remain ongoing. Production deployment should put HTTPS in front of the service, restrict the local first-wallet bootstrap channel at the network layer, and maintain external database backups or audit checkpoints.

## PengMiner desktop

The Tauri 2 Windows desktop miner lives in [`apps/pengminer`](apps/pengminer).
It links the existing PengWallet mining library and provides a React dashboard,
CPU/OpenCL controls, wallet login, and live measured statistics. See its
[setup, build, and validation instructions](apps/pengminer/README.md).
