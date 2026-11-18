# PengChain single-file web miner

`index.html` contains the complete frontend: HTML, light-theme CSS, protocol
implementation, controller, and Blob Web Workers. It has no external runtime
assets, packages, framework, bundler, or frontend build step. The other files
are deployment tooling, documentation, and tests, not browser dependencies.

## Use

Host the file on the **same HTTPS origin** as the PengChain API, at
`/web-miner/`. Sign in using an existing wallet ID and password. Choose the
worker count and the server's exact `NETWORK_ID` (default `pengchain-v1`), then
click **Start Mining**. One worker is the conservative default; the configurable
limit is the smaller of 16 and the browser-reported hardware concurrency.

The page checks an existing browser session on load without requesting a job or
starting workers. The balance API does not return the wallet ID; for an existing
cookie, its identity is displayed when the first user-requested job arrives.
Signing in through the form displays the wallet ID immediately. A native app's
OS-protected session is not a browser cookie; sign in separately in your browser.

Stop cancels the loop, fetches, backoff timers, and workers. Hiding the tab,
leaving the page, or detecting a long browser sleep also stops mining; resuming
always requires another click. No service worker or background miner is installed.

## Verified integration

The implementation follows `server/src/pca10.ts`, `server/src/app.ts`,
`client/src/mining.rs`, and `docs/protocol.md`:

- The six fields are `[one raw version byte (1), UTF-8 network, UTF-8 job ID,
UTF-8 token ID, UTF-8 wallet ID, decoded 32-byte base64url challenge]`. Each
  has an unsigned four-byte big-endian byte-length prefix. Append an unsigned
  eight-byte big-endian nonce. Ten SHA-256 rounds hash raw bytes/digests.
- Difficulty counts leading hexadecimal zero digits, including odd nibbles.
  Worker `i` of `N` searches `i + N*k` using BigInt and stops before uint64
  overflow. Every counted hash is a completed ten-round PCA-10 evaluation.
- `GET /blockchain/api/mining/job?symbol=PENG`, with the session cookie, returns
  `jobId`, `tokenId`, `symbol`, `walletId`, `challenge`, `difficulty`, `issuedAt`,
  `expiresAt`, and `reward`. Jobs expire after 300,000ms. Version and network ID
  are not returned by this API: version is fixed at 1, and the operator must
  configure the matching network ID in the UI.
- `POST /blockchain/api/mining/submit`, JSON `{jobId, nonce}`, uses a decimal
  nonce string. Success is `{accepted:true, reward, digest, difficulty}`.
  Only this response increments accepted jobs and confirmed run earnings.
  Job rewards are **not fixed at 10**: PENG's owner-configured range defaults
  to 5–15 integer base units. No monetary policy is changed here.
- `GET /blockchain/api/wallet/balance` returns an authenticated array of
  `{tokenId, symbol, amount}`. An absent PENG row means zero. Balances are never
  edited locally and values outside safe JSON integer precision are rejected.
- `POST /blockchain/api/wallet/login`, JSON `{walletId,password}`, uses the
  server's ordinary 24-hour HttpOnly, SameSite=Strict `peng_session` cookie
  (Secure in production). `POST /blockchain/api/wallet/logout` revokes it.
  No credentials are sent to mining endpoints, logged, or saved in localStorage,
  sessionStorage, IndexedDB, or worker messages. Password input is cleared on
  submission; the password is used only for the login request.

The checked-in server has **no TOTP/2FA endpoints, separate browser login page,
or CSRF-token protocol**. This page neither invents nor bypasses one. All API
requests use `credentials: same-origin`, `mode: same-origin`, JSON POSTs, and
reject redirects. Existing cookie and security middleware remain unchanged.
If your deployed installation adds an authentication/2FA gateway, complete its
normal flow first and ensure it permits these same-origin routes; this page
does not accept TOTP secrets. A deployment with a different application-level
2FA/CSRF protocol needs an explicit integration before this page can support it.

Temporary failures use abortable 15-second request timeouts and exponential
backoff from 1 to 30 seconds, stopping after eight retries. HTTP 429 honors
Retry-After up to 120 seconds. A run requests no more than 30 jobs/minute,
refreshes balances at most every 15 seconds after receipts, and does not poll
while idle. This leaves room under the existing 100 requests/minute limit;
other tabs/clients sharing an IP can still cause rate limiting. Stale/duplicate
submissions are logged and followed by fresh jobs; invalid proofs or other
permanent failures stop with an explanation.

A submission whose response is lost **is never retried by this controller**:
the server may already have committed it. The next job is fresh and no reward
is added to confirmed run earnings without an acceptance receipt. Use Refresh
balance after stopping to reconcile authoritative funds. The server rejects
replays and cannot pay a job twice. Session totals can therefore undercount
funds after lost responses, but never infer rewards from a balance difference.

## Hosting on the existing NAS / HTTPS proxy

No backend or ledger changes are needed. Serve only the HTML at the existing
public API origin through the NAS's static-file/reverse-proxy layer. Keep the
existing API proxy, TLS termination, cookie flags, Cloudflare access controls,
and rate limits. Do not serve the miner directly from `file://` or a separate
public origin. Localhost HTTP is supported only for local development/tests.

Example Nginx locations **inside the existing HTTPS server block** (replace
filesystem paths; retain your current API `proxy_pass` and TLS settings):

```nginx
location = /web-miner { return 308 /web-miner/; }
location = /web-miner/index.html { return 308 /web-miner/; }
location = /web-miner/ {
    alias /srv/pengchain/web-miner/index.html;
    default_type text/html;
    include /etc/nginx/peng-web-miner-csp.conf;
    add_header Cache-Control "no-store" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header Referrer-Policy "same-origin" always;
}
# Your existing /blockchain/api/ proxy stays in place.
```

Generate the CSP include from the **exact deployed file**:

```sh
node web-miner/csp.mjs > /tmp/peng-web-miner-csp.conf
# Install the HTML and this include into the paths above using your NAS's
# normal deployment process; validate with nginx -t before reloading.
```

The generator permits only hashes of this file's inline scripts and CSS,
same-origin connections, and Blob workers. It forbids frames, external scripts,
objects, and base URL injection. It requires neither `unsafe-eval` nor
`unsafe-inline`. Regenerate the hashes whenever the HTML changes; do not
reformat/minify the deployed copy after generating them. Preserve equivalent
TLS/HSTS and other proxy policies in this location: Nginx `add_header` inheritance
may require restating your existing server-level headers here.

The existing Fastify Helmet policy blocks inline scripts. Do not disable it or
relax the API policy globally: use the dedicated static location and generated
hash policy for this page. If there are multiple CSP headers, **all** apply;
ensure the page's proxy location receives the intended hash policy rather than
the API's default inline-blocking policy. Its API calls still pass through the
unchanged server middleware. For Docker/Cloudflare deployments, the static
location belongs in the proxy container/NAS layer in front of `pengchain:3000`;
mount the one HTML file into that proxy. The server image need not be rebuilt.

Cross-origin deployment is intentionally unsupported. The current server has
no CORS configuration; the Strict cookie and same-origin frontend do not permit
this usage. A separate-origin design would require an exact trusted-origin
allowlist, credentialed preflight/method/header handling, and compatible cookie
and CSRF policy. **Never use wildcard credentialed CORS or weaken cookies** to
make a separate-origin copy work. Same-origin hosting avoids those changes.

## Tests

Use an isolated environment, never production wallet funds:

```sh
npm ci --prefix server
npm run build --prefix server
npm test --prefix server
node web-miner/tests/protocol.test.mjs
npm ci --prefix apps/pengminer
cd apps/pengminer && npx playwright install --with-deps chromium && cd ../..
node web-miner/tests/browser.mjs
```

The existing app's Playwright dev dependency is used **only for tests**. For a
system Chromium installation, set `CHROMIUM_PATH=/usr/bin/chromium`. Set
`SCREENSHOT_DIR` to a directory to save desktop/mobile captures.

Protocol tests extract the shipped embedded core, compare canonical bytes
against the server (including UTF-8 and all uint64 boundaries), assert exactly
ten raw-digest rounds, check valid/invalid Rust vectors, odd-nibble difficulty,
worker lane disjointness/overflow, and wall/monotonic expiry. Browser tests run
real Blob workers under the generated strict CSP against the unchanged Fastify
app and temporary SQLite database. They test the Rust nonce vector in a worker,
real accepted/rejected/replayed submissions, confirmed balances, stale/expired
jobs, temporary failures/backoff, a committed-but-lost acceptance response,
Stop/offline/hidden-tab cancellation, session revocation, cookie restoration,
no automatic mining, and mobile overflow. The workflow in
`.github/workflows/web-miner.yml` runs these checks on relevant PRs/main pushes.

## Limitations

Web Crypto's asynchronous digest calls are slower than native Rust CPU/OpenCL
mining; browsers can throttle workers. Worker telemetry is reported every
200ms and flushed on orderly cancellation. If a worker is forcibly terminated
after the 1.5-second cleanup timeout, the log states that its unreported hashes
were not counted; no hash counts are estimated. Totals reset for each explicit
Start and are not persisted. Balance is a timestamped server snapshot, not a
live promise of credit. The job lifetime is mapped from server Date/issuedAt
onto local wall and monotonic deadlines; the server always makes the final
expiry decision. No wallet creation, admin, transfer, custom-token mining,
service worker, GPU mining, or session export is included.
