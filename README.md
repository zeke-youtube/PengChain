# PengChain V1

PengChain is a centralized proof-of-work ledger by PikaStudio. The server is authoritative and persists wallet accounts, sessions, tokens, balances, transfers, mining jobs, rewards, and tamper-evident audit records in SQLite WAL mode.

Run the server with `cd server`, `npm install`, and `npm run dev`. Set `PENGCHAIN_DB`, `PORT`, `HOST`, and `NETWORK_ID` through the environment. The public network website is served at the server root, for example `http://127.0.0.1:3000/`. Docker deployment is defined in `server/compose.yaml` and persists the database in a named volume.

## Publish the NAS server through Cloudflare

Use Cloudflare Tunnel so the QNAP does not need an inbound port exposed. Create a tunnel in Cloudflare Zero Trust, add a public hostname that points to the `pengchain` service on port `3000`, then save the tunnel token in `server/.env`:

```text
CLOUDFLARE_TUNNEL_TOKEN=your-tunnel-token
```

Start the server and tunnel together from `server/`:

```bash
docker compose -f compose.yaml -f compose.cloudflare.yaml up -d
```

The tunnel service waits for the PengChain health check and reconnects after a restart. Do not commit `.env` or the tunnel token. The example locally-managed configuration is in `server/cloudflared-config.example.yml`.

The PengWallet Rust CLI lives under `client/`; see its [development instructions](client/README.md). The server acceptance matrix and other V1 work described in `AGENTS.md` remain ongoing. Production deployment should put HTTPS in front of the service, restrict the local first-wallet bootstrap channel at the network layer, and maintain external database backups or audit checkpoints.

## PengMiner desktop

The Tauri 2 Windows desktop miner lives in [`apps/pengminer`](apps/pengminer).
It runs the separately bundled PengWallet CLI process and provides a React dashboard,
CPU/OpenCL controls, wallet login, and live measured statistics. See its
[setup, build, and validation instructions](apps/pengminer/README.md).

## Android miner

The native Android PENG CPU miner, build/install instructions, and verification
steps are in [apps/pengminer-android](apps/pengminer-android/README.md).
