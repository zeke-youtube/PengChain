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

After creating a wallet and logging in, `pengwallet mine PENG` requests one job,
searches real unsigned 64-bit nonces using PCA-10 on the CPU, and submits the
first valid solution. It reports success only after the server accepts it.
Expired jobs and rejected submissions exit with an error; rerun to get a new job.
The server must use the matching network ID: pass `mine PENG --network ID` or set
`NETWORK_ID` (default `pengchain-v1`). No CUDA backend is provided.
