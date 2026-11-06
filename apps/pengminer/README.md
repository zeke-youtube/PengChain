# PengMiner V1

A Tauri 2 desktop application for the centralized PengChain ledger. React renders
the dashboard; Rust handles authentication, devices, mining, and server requests.
The app links PengWallet's library directly. It never launches a CLI miner or
changes the mining protocol. No keys, blocks, distributed consensus, or fake
rewards are introduced.

## Windows setup

Install Node.js 22+, stable Rust for `x86_64-pc-windows-msvc`, Visual Studio 2022
Build Tools with **Desktop development with C++** and a Windows SDK, and the
Microsoft Edge WebView2 runtime. For GPU mining, install the GPU vendor's OpenCL
driver. An NVIDIA RTX 5070 Ti must be exposed as an available OpenCL device by
its installed driver; CUDA is not required or used.

From the repository root:

```powershell
cd server
npm ci
npm run build
npm start
# In a second terminal:
cd apps/pengminer
npm ci
npm run tauri dev
```

Use an existing PengChain wallet to sign in. The default server is
`http://127.0.0.1:3000`; remote servers require HTTPS. Settings allow choosing a
server origin, network ID, token, device, CPU thread count, and intensity. Select
your GPU by its driver-reported name, then click **START MINING**. **STOP MINING**
cancels work and waits for CPU threads or the current GPU dispatch to finish.

A browser-only `npm run dev` intentionally disables mining and sign-in and shows
that the native backend is unavailable. It does not simulate mining data.

## Windows installer

```powershell
cd apps/pengminer
npm ci
npm run tauri build -- --bundles nsis
```

WebView2 must already be installed: the installer does not download or execute
a WebView2 bootstrapper. The configuration creates a per-user NSIS installer at
`src-tauri/target/release/bundle/nsis/`. The GitHub workflow
`.github/workflows/pengminer-windows.yml` runs checks and builds/uploads that
installer on a Windows runner. It is not an auto-update or download mechanism.
Installers are unsigned until the project configures a signing certificate.
No Windows installer has been built or tested in the Linux development cloud.

## Linux development

Install distribution packages for `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libayatana-appindicator3-dev`, `librsvg2-dev`, `libdbus-1-dev`, `pkg-config`, and
build tools. Then use the same npm/Tauri commands. Linux saved sessions require
a running Secret Service (for example GNOME Keyring). The cloud's extracted
library sysroot is a local testing aid and is not a repository requirement.

## Behavior and security

- CPU thread counts are bounded by available logical threads. Nonce lanes do not
  overlap. CPU intensity is a duty-cycle target (25/50/100 percent); GPU intensity
  changes nonce batch size and inter-batch pauses, not a claimed hardware power cap.
- GPU enumeration reports name, vendor, OpenCL GPU index, memory, availability,
  and host-unified-memory information. Memory is total global device memory, not
  a fabricated free-VRAM measurement. The default GPU preference favors a device
  without shared host memory; this is a heuristic, not a guarantee of dedicated
  hardware. Shared-memory GPUs can be selected explicitly. There is no CPU fallback.
- The existing PCA-10/OpenCL kernel is reused. CPU verification of a GPU result
  remains in place before the server performs its own proof verification.
- Hashrate counts actual completed PCA-10 evaluations. Current rate uses a 500 ms
  window; average rate includes the whole mining session, including network waits.
  Final counters are read after stop. Uptime and session earnings reset on start.
- Each job is submitted once. Network failures and rejected submissions stop
  mining. Unknown outcomes are not replayed. A request already sent when Stop is
  clicked may still be accepted; the server balance is authoritative.
- Rewards and balances are integer base units. History is local acknowledged
  receipts (latest 200 for the current wallet/server, 1000 total), not an invented
  server-wide transaction history. Logs retain at most 150 runtime messages.
- Passwords are used for login only. Cookies remain in Rust and are never sent to
  React, events, logs, or settings files. Sessions use Windows Credential Manager,
  macOS Keychain, or Linux Secret Service. If secure storage is unavailable, login
  remains in memory only and the UI explains that restart requires signing in again.
- Sessions are scoped to the exact validated server origin. Redirects are disabled
  so credentials cannot be forwarded to another origin. Credential-bearing URLs
  and remote plaintext HTTP are rejected. Mining requires a validated session.
- The desktop uses a single-instance guard as well as an engine session lock.
  Launching it again focuses the existing window instead of creating a second miner.
- Start-on-launch is off by default. Enabling it in Settings is explicit consent;
  it runs only when a saved session can be validated on launch. There is no OS
  autostart registration. Close-to-tray is separately opt-in; tray Stop/Quit remain
  available. Notifications are also off by default.
- The webview has a restrictive CSP, event-listening permissions, and narrow Rust
  commands. It has no shell, filesystem, process-launch, or arbitrary HTTP plugin.
- PPU detection, USB transport, and PPU mining are **not implemented**. The card is
  informational and the backend rejects PPU device settings.

## Tests

Build the server first and put Node 22 on PATH. From the repository root:

```sh
npm --prefix server ci
npm --prefix server run build
cargo test --locked --manifest-path client/Cargo.toml
cargo test --locked --manifest-path apps/pengminer/src-tauri/Cargo.toml --no-default-features
cd apps/pengminer
npm ci
npm run typecheck
npm run lint
npm test
npm run build
npm run tauri build -- --debug --no-bundle
```

The Rust integration test starts an isolated real server and SQLite database. It
covers login failure/success, session-store persistence/restoration, wallet balances,
real CPU rewards, singleton start, events, stop, hard-job cancellation, settings,
GPU selection policy, and network failure. Credential storage is dependency-injected
for deterministic tests; real Windows Credential Manager behavior requires Windows.
Frontend tests cover IPC login, event rendering and cleanup, no implicit start,
configuration locking, errors, chart bounds, and the disabled PPU.

The shared library's opt-in OpenCL conformance test can run on a GPU-equipped host:

```sh
cd client
PENGCHAIN_TEST_OPENCL_DEVICE=gpu cargo test --locked \
  gpu::tests::opencl_kernel_matches_rust_sha256 -- --ignored --nocapture
```

Software PoCL can execute that test with `PENGCHAIN_TEST_OPENCL_DEVICE=cpu` to
check the real kernel's digests/search results. This is test-only and cannot
activate CPU fallback in GPU mining.

## Validation limits

Windows/WebView2, Windows credential persistence, the RTX 5070 Ti, physical GPU
performance, tray behavior on Windows, notifications on Windows, and the NSIS
installer require validation on a Windows/GPU machine. No Windows installer or
physical GPU test is claimed by Linux results. See `VALIDATION.md` for the actual
checks completed for this version.
