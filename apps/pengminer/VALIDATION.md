# PengMiner V1 validation

Validated in the Linux cloud on 2026-11-06. These are observed results, not claims
about Windows or physical GPU execution.

| Check | Result |
| --- | --- |
| React/TypeScript production build (`npm run build`) | Passed |
| TypeScript (`npm run typecheck`) | Passed |
| ESLint (`npm run lint`) | Passed |
| React/Vitest tests | 5 passed |
| Desktop Rust integration tests, without the GUI feature | 4 passed |
| Strict desktop Clippy, including native code and all targets | Passed |
| Tauri build with embedded production assets, debug, no bundle | Passed; Linux executable produced |
| Native window startup/rendering on Xvfb | Passed with isolated Linux library mounts |
| Native UI login → Start → real server reward → Stop | Passed; 3 accepted solutions, 30 PENG base units, no new solutions after Stop |
| Browser rendering/navigation | All six pages rendered; browser-only mode correctly disables native operations |
| Shared PengWallet library tests | 6 passed; hardware-dependent OpenCL test is opt-in |
| Explicit OpenCL kernel conformance with software PoCL | Passed, including GPU-batch work counters and intensity |
| Existing CLI real-server smoke test | Passed, including auto-login, transfers, repeated rewards, Ctrl+C and JSON output |
| Independent CLI mining HTTP smoke test | Passed |
| Existing TypeScript server build/tests | Build passed; 5 tests passed |
| Windows Credential Manager round-trip test | Added under `cfg(windows)`; not run in Linux |
| Windows/WebView2 runtime, tray and notifications | Not run |
| RTX 5070 Ti / other physical GPU | Not available; not run |
| NSIS Windows installer | Build configuration and Windows CI provided; installer not built here |
| PPU detection/mining | Intentionally not implemented or enabled |

The backend integration tests start an isolated real PengChain server and SQLite
file. They exercise actual proof computation and rewards, authentication failures,
balance retrieval, persistence through a dependency-injected credential store,
restart without automatic mining, singleton start, active CPU cancellation, event
emission, GPU selection policy and driver absence, network failure, and rejected
submission handling. No fake mining results are used by the product.

The native UI check used actual mouse/keyboard input under Xvfb and checked the
server database independently of the displayed counters. WebKit/GTK development
libraries were extracted from signed Debian packages into a workspace-local
sysroot; read-only mount mappings supplied WebKit's expected subprocess paths.
This is a cloud testing accommodation, not a required installation procedure.
No WebKit sandbox bypass was added to the application.

On this machine the OS Secret Service was unavailable; the native app correctly
used an in-memory login and reported that persistence was unavailable. Real
Windows secure storage is tested only by the Windows-specific test when executed
on a Windows host.

Use the Windows workflow or the README's build instructions for platform
validation. Installer signing and physical GPU throughput are not claimed.
