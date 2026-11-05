# PengChain V1 API

The server exposes `/health`, wallet creation/login/logout, balances, token registry/search, transfers, mining jobs/submissions/stats, and public audit logs under `/blockchain/api`.

Authentication uses an HttpOnly SameSite cookie named `peng_session`. Production deployments must terminate HTTPS at a trusted reverse proxy or directly in front of the server.

Wallet creation and token creation accept JSON. Transfer amounts are integer base units. Mining nonces are decimal strings so clients do not lose 64-bit precision.
