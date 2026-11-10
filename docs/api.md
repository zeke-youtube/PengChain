# PengChain V1 API

The server exposes `/health`, wallet creation/login/logout, balances, token registry/search, transfers, mining jobs/submissions/stats, and public audit logs under `/blockchain/api`.

Use `pengwallet logs` after login to read audit records from the CLI. Filters include `mining`, `reward`, `block`, `tx`, `ban`, and `token`. The compatibility alias `block` maps to accepted mining records. The log payload includes the event type, server timestamp, event details, and the hash-link proof prefix.

Owners can read or update the PENG reward range with the authenticated `GET` and `PUT /blockchain/api/admin/peng-reward-range` endpoints. The request body is `{ "minimum": 5, "maximum": 15 }`. Both values are integer base units, and the server validates the range and records the change in the audit log.

Authentication uses an HttpOnly SameSite cookie named `peng_session`. Production deployments must terminate HTTPS at a trusted reverse proxy or directly in front of the server.

Wallet creation and token creation accept JSON. Transfer amounts are integer base units. Mining nonces are decimal strings so clients do not lose 64-bit precision.

Native apps can send `rememberMe: true` to `POST /blockchain/api/wallet/login`.
The login response includes `persistent: true` for this mode. The server stores a
session without automatic expiry (the SQLite integer sentinel is JavaScript's
maximum safe integer), and the native app keeps the cookie in OS-protected
storage until logout, revocation, or app-data removal. The response cookie has a
long browser max-age; native session storage is independent of browser expiry.
Omitting `rememberMe` retains the ordinary 24-hour session policy. Logout deletes
the session, including remembered sessions. This requires deploying the updated
server; old servers may ignore the optional field and retain their 24-hour expiry.
