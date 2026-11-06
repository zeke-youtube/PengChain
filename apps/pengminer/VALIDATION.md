# PengMiner CLI-process architecture validation

Observed in Linux cloud on 2026-11-06. PengMiner launches the separate PengWallet
CLI; the desktop no longer links the mining implementation.

| Check | Observed result |
| --- | --- |
| Desktop frontend build, TypeScript, ESLint | Passed |
| React tests | 5 passed |
| Desktop subprocess integration tests | 6 passed |
| CLI library regression tests | 6 passed; physical OpenCL test remains opt-in |
| CLI subprocess protocol smoke test | Passed: real rewards, telemetry, Stop, stdin EOF cancellation, no session file |
| Existing CLI real-server and mining smoke tests | Passed |
| Strict Clippy for CLI and complete native desktop | Passed |
| Native Tauri build with prepared CLI sidecar | Passed on Linux |
| Actual native-window login → Start → Stop | 3 accepted solutions, 30 PENG; no new rewards after Stop |
| Separate process ownership | Exactly one CLI engine observed; no credentials in arguments; child exited and was reaped on Stop |
| Installer input consistency | Portable CLI, target-suffixed sidecar, and adjacent development engine were byte-identical |
| Desktop dependency graph | No `pengwallet` mining crate or `opencl3`; only the data-only protocol crate is shared |
| Missing/legacy engine and forced termination | Passed, including termination/reaping after ten-second stop timeout |
| Updated Windows portable/NSIS outputs | Configured for the Windows workflow; current build/release status is pending |
| Physical GPU and Windows-specific runtime behavior | Require the relevant hardware/OS; not claimed by Linux results |

The Windows workflow builds the standalone portable CLI and the app installer
containing that same CLI, uploads both artifacts, and publishes both to a GitHub
Release after successful builds on main. Pull-request runs do not publish releases.
Current remote build/release status must be read from GitHub Actions rather than
inferred from local results.

The previous direct-library native smoke evidence is superseded by an actual
subprocess native check: a separate CLI PID computed and submitted the proofs;
its process disappeared after Stop. Linux GUI testing used Xvfb and isolated
read-only WebKit library mounts without adding a sandbox bypass to the product.
The Linux credential service was unavailable, and the app used its explicit
in-memory session fallback. Windows Credential Manager has its own Windows-only
round-trip test in the workflow.
