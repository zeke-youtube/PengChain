# PengWallet development

Build with `cargo check`, `cargo test`, and `cargo build --release` from `client/`.
After building the release binary, run `python3 tests/mining_smoke.py` for an
HTTP protocol test with an independent proof-of-work verifier and rejection case.
Run `target/release/pengwallet --version` to see the package version and embedded
UTC compile timestamp. `build.rs` uses Chrono's UTC clock when Cargo executes the
build script; launching the binary never recalculates this value.

Cargo caching can reuse a previous build script output and its timestamp, even
when application sources change. For a fresh release timestamp, run:

```sh
cargo clean -p pengwallet --release
cargo build --release
target/release/pengwallet --version
```

After creating a wallet (which automatically signs you in), `pengwallet mine PENG` continuously
requests fresh jobs, searches real unsigned 64-bit nonces with CPU PCA-10, and
submits valid solutions. Press **Ctrl+C** to stop, including during CPU work or
between jobs. The final summary counts only acknowledged rewards for this run.
A submission already sent when interrupted may still be accepted by the server;
check `pengwallet balance` for the authoritative balance.

```sh
pengwallet mine PENG                 # Continuous until Ctrl+C
pengwallet mine PENG --once          # Stop after one accepted reward
pengwallet mine PENG --jobs 5        # Stop after five accepted rewards
pengwallet mine PENG --json          # One accepted JSON receipt per line
```

Expired jobs are replaced automatically. A two-second pause between jobs keeps
requests below the server's default rate limit. Rejected submissions, lost
connections, or expired sessions stop the miner with an error; resolve the issue
and restart it. No automatic replay or reward is invented. `--jobs` counts accepted
rewards and must be positive; it cannot be combined with `--once`.
The server must use the matching network ID: pass `mine PENG --network ID` or set
`NETWORK_ID` (default `pengchain-v1`).

## Terminal interface

Commands show readable wallet details, balance columns, token cards, and transfer
receipts by default. Mining reports progress on stderr and shows the reward only
after the server accepts it. Token amounts remain integer base units; grouping
separators are for display only. Errors go to stderr and return a nonzero status.

```sh
pengwallet balance
pengwallet token-search PENG
pengwallet mine PENG
pengwallet send PENG PENG-RECIPIENT 3
```

Use `--json` on any command for machine-readable stdout (JSON errors go to stderr).
Mining progress is silent in this mode. `--no-color` or `NO_COLOR=1` disables
colors; redirected output is also uncolored. See `pengwallet --help` for commands
and examples. Server addresses and these output flags can go before or after a
subcommand.

Run the real-server UI smoke test after building both projects, with Node 22 on
PATH: `python3 tests/cli_smoke.py` from `client/`. It creates an isolated temporary
SQLite database and sessions, exercises wallet creation/login, search, mining,
transfers, balance, logout, readable errors, and JSON mode, then stops its server.


## GPU mining (OpenCL)

```sh
pengwallet mine PENG --gpu                  # Continuous GPU mining
pengwallet mine PENG --gpu --once           # One GPU reward
pengwallet mine PENG --gpu --jobs 5 --json   # Five GPU rewards, JSON lines
pengwallet mine PENG --gpu --gpu-device 1   # Select the second OpenCL GPU
```

The Rust host dispatches a real PCA-10 OpenCL compute kernel: each work item hashes
one nonce ten times, using the same length-prefixed input, big-endian nonce, raw
digests, and hexadecimal difficulty as the CPU and server. Work is dispatched in
16,384-nonce batches. Returned solutions are verified on the CPU before submission;
this is a correctness check, not a CPU substitute for GPU searching. The compiled
kernel and selected device are reused across jobs.

Install your GPU vendor's OpenCL runtime (OpenCL 1.2 or newer, with its kernel
compiler). Compatible NVIDIA, AMD, and Intel GPUs can use this backend when their
drivers expose them through OpenCL. This is an OpenCL backend, not CUDA. On Linux,
the OpenCL ICD loader and vendor ICD are both required; on Windows the driver must
provide OpenCL.dll. No OpenCL SDK is required to build the Rust binary because
the driver is dynamically loaded. Containers/headless hosts must also expose the
GPU devices and driver libraries. Use `clinfo` to inspect installed OpenCL devices.

CPU remains the default. `--gpu` selects GPU devices only and fails clearly if the
requested device or runtime is unavailable; it never falls back to CPU. Device
indices enumerate GPUs, not CPU OpenCL devices. Ctrl+C and expiry are checked
between GPU batches; an in-flight driver compilation or kernel dispatch must
finish before its resources can be released. Driver errors stop mining.

### GPU validation

Normal `cargo test` does not require a GPU. Run the explicit kernel conformance
check on a GPU-equipped host:

```sh
PENGCHAIN_TEST_OPENCL_DEVICE=gpu cargo test --locked \
  gpu::tests::opencl_kernel_matches_rust_sha256 -- --ignored --nocapture
```

For a software-only kernel correctness check, install PoCL and set
`PENGCHAIN_TEST_OPENCL_DEVICE=cpu` for that same command. CPU selection exists only
in the test; it cannot enable a CPU fallback through the CLI. The test compares
OpenCL digests against Rust SHA-256 across SHA padding boundaries, high nonces,
and difficulty values, then checks search results, cancellation, and expiry.

For real GPU-to-server reward acceptance, build both projects, put Node 22 on
PATH, and run `PENGCHAIN_TEST_GPU=1 python3 tests/cli_smoke.py` from `client/`.
This additionally requires three GPU-mined rewards to be accepted by a real server
using a temporary database.

The development cloud instance has no exposed physical GPU. Kernel conformance
has been verified with software OpenCL; physical GPU execution and GPU-to-server
acceptance remain to be validated on a GPU-equipped machine. No GPU performance
or hardware compatibility measurements have been made.


## Create and sign in

`pengwallet create` prompts for a password, creates the wallet, and automatically
signs in to it. Only the session cookie and wallet ID are saved for later commands;
the password is not saved. You can immediately run `pengwallet balance` or
`pengwallet mine PENG`. Creating another wallet switches the saved session to that
new wallet. Existing wallets can still use `pengwallet login`.

JSON creation output includes `walletId`, `role`, and `signedIn: true` after the
session has been saved. If the wallet is created but automatic login or saving the
session fails, the command exits with an error containing the new wallet ID and
manual login instructions. Do not repeat creation to recover that wallet.
