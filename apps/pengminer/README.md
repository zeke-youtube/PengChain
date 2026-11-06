# PengMiner V1

PengMiner is a Tauri 2 / React desktop controller for the **PengWallet CLI**.
Mining runs in a separate `pengwallet.exe` process, not inside the desktop binary.
The Windows app installer includes the matching CLI executable. Installing a
second standalone CLI is optional.

## Two Windows distributions

| Output              | Contents                     | Installation                                                     |
| ------------------- | ---------------------------- | ---------------------------------------------------------------- |
| PengWallet portable | `pengwallet.exe`             | No installer. Run it from a terminal.                            |
| PengMiner desktop   | A per-user NSIS setup `.exe` | Installs PengMiner **and** its separate `pengwallet.exe` engine. |

The Windows workflow uploads `PengWallet-Windows-portable` and
`PengMiner-Windows-NSIS` as separate artifacts. There is no CLI installer.
Windows artifacts have not been built in the Linux cloud; see `VALIDATION.md`.

## Windows development and packaging

Install Node.js 22+, stable Rust for `x86_64-pc-windows-msvc`, Visual Studio 2022
Build Tools with Desktop development with C++ and a Windows SDK, and the Microsoft
Edge WebView2 runtime. GPU mining requires the vendor's OpenCL driver. No CUDA
backend or runtime downloader is used.

From the repository root:

```powershell
# Portable CLI, without an installer:
cargo build --release --locked --manifest-path client/Cargo.toml
# Output: client/target/release/pengwallet.exe

# Run the local ledger server:
cd server
npm ci
npm run build
npm start
# In a separate terminal, from the repository root:
cd apps/pengminer
npm ci
npm run tauri dev

# Build the desktop installer, including its CLI engine:
npm run tauri build -- --bundles nsis
# Output: src-tauri/target/release/bundle/nsis/*.exe
```

The Tauri pre-build/pre-dev hook runs `npm run prepare-engine`. It builds PengWallet
from repository source, copies a target-suffixed binary into `src-tauri/binaries/`,
and prepares an adjacent engine for native development/no-bundle runs. Tauri's
`externalBin` configuration includes that executable in the installer next to
PengMiner. Generated engine files, targets, frontend output, and installers are
ignored by Git. No arbitrary engine is downloaded.

The Windows workflow builds and uploads the portable CLI before running desktop
integration tests, then builds/uploads the NSIS installer. WebView2 must already
be installed: runtime bootstrapping is disabled. Installers are unsigned until
the project configures code signing.

## Engine selection and process security

By default, PengMiner resolves the included `pengwallet.exe` next to its own
executable. It does not search PATH or execute a shell. Settings includes an
optional **absolute path to a trusted PengWallet executable** for a separately
installed portable CLI. Leave it blank to use the included engine. The executable
must support the matching versioned JSON engine protocol.

The desktop does not depend on `pengwallet`, OpenCL, or the mining kernel. Its only
shared Rust crate is `crates/pengwallet-protocol`, containing IPC data types and GPU
selection policy, not proof computation. All CPU/OpenCL searching, job requests,
and reward submissions happen in the CLI process.

The desktop handles login and OS-backed session storage. It passes a session to
the engine using a private stdin pipe, never command-line arguments, environment
variables, or plaintext session files. JSON-line events report actual job details,
measured hashes, solutions, server acknowledgements, errors, and stop summaries.
Frames are size-bounded and protocol-version checked. Stderr is drained without
copying raw output into UI logs. The GUI does not scrape human terminal output.

Stop sends an explicit JSON stop command to the CLI, then waits for the process.
The CLI joins CPU workers and finishes an in-flight GPU dispatch; submitted HTTP
requests are read once and never replayed. If the CLI does not stop within ten
seconds, PengMiner terminates/reaps it and reports that fact. Closing the private
stdin pipe also requests cancellation. A submitted request may still be accepted
by the server even if its response is lost: the wallet balance is authoritative.

The UI remains responsive, and singleton/session guards prevent accidental parallel
mining. No broad shell, filesystem, process, or HTTP webview plugin is enabled.

## UI and wallet behavior

Use an existing PengChain wallet to sign in. Configure a server origin, token,
network ID, CPU/GPU device, CPU threads, and intensity. Remote servers require
HTTPS; loopback HTTP supports local development. Select the driver-reported GPU
name (for example an RTX 5070 Ti) and click START MINING. GPU failures never silently
fall back to CPU. Shared-memory/integrated GPUs must be explicitly selected when
no dedicated-device preference can be satisfied.

Hashrate is completed PCA-10 evaluations reported by the CLI. Current rate uses
500 ms samples, and average rate includes job/network waits. The chart uses actual
sample timestamps. CPU intensity controls duty cycle; GPU intensity adjusts batch
size and inter-batch pauses, not a hardware power cap. OpenCL memory is reported
global device memory, not a fabricated free-VRAM measurement.

Passwords are never saved. Sessions use Windows Credential Manager, macOS Keychain,
or Linux Secret Service where available; otherwise login stays in memory only and
the app reports that restart requires sign-in. Session cookies never enter React,
settings, events, logs, or the CLI's ordinary session file. Sessions are scoped to
the exact server origin, and HTTP redirects are disabled.

Start-on-launch, close-to-tray, and notifications are opt-in and off by default.
Start-on-launch requires a valid saved session. There is no OS autostart registration.
Tray Stop/Quit remain available when the window is hidden. History retains local
server acknowledgements (200 per wallet/server, 1000 total), while the server owns
all balances and rewards. PPU detection, USB transport, and mining remain explicitly
**Not implemented** and disabled.

Browser-only `npm run dev` renders the interface but intentionally disables native
operations. It never fabricates mining data.

## Tests and Linux development

Install Node 22 and Rust. Native Linux builds need `libwebkit2gtk-4.1-dev`,
`libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libdbus-1-dev`,
`pkg-config`, and build tools. Secure persistence needs a running Secret Service.

From the repository root:

```sh
npm --prefix server ci
npm --prefix server run build
cargo build --release --locked --manifest-path client/Cargo.toml
cargo test --locked --manifest-path client/Cargo.toml
cargo test --locked --manifest-path apps/pengminer/src-tauri/Cargo.toml --no-default-features
cd apps/pengminer
npm ci
npm run typecheck
npm run lint
npm test
npm run tauri build -- --debug --no-bundle
```

Desktop tests use the real portable CLI (default `client/target/release/pengwallet`,
`.exe` on Windows). `PENGWALLET_ENGINE` can select the test executable. Tests cover
actual mining rewards, persistence/restoration, duplicate start, cancellation,
rejected submissions, telemetry, device discovery and policy, and network failures.
The CLI retains its existing regression/conformance tests. Physical GPU testing
and Windows Credential Manager/WebView2/tray/installer execution require the relevant
host; no physical GPU or Windows installer is claimed by Linux test results.

## Automatic GitHub Releases

Relevant pushes/merges to `main` automatically run the Windows workflow. After the
build and CI checks succeed, a separate job publishes both `.exe` downloads to a
new GitHub Release and marks it Latest. Pull requests build artifacts but never
publish releases. A manual workflow run on `main` can also publish a release.

Release tags include the app version, workflow run number, and run attempt, for
example `pengminer-v1.0.0-build.8.1`, so existing tags/assets are not overwritten.
The workflow first uploads both assets to a draft and publishes only after both
uploads succeed. Only the release job receives `contents: write`; no personal
access token or repository secret is needed. Publishing a release does not install
or automatically execute either binary on users' machines.
