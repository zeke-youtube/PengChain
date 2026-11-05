# PengChain V1 Agent Guide

## Project purpose

PengChain is a centralized, server-authoritative proof-of-work ledger for PikaStudio.

It is not a blockchain. Do not add blocks, chained hashes, peer-to-peer networking, distributed consensus, or public/private-key wallets. The server owns wallet accounts, sessions, token definitions, balances, transfers, mining jobs, rewards, difficulty, and audit history.

## Current state

- Git is initialized at the repository root.
- The TypeScript server exists under `server/`.
- The server has SQLite WAL persistence, Argon2id password hashing, Fastify routes, cookie sessions, token registration and search, transfers, PCA-10 verification, mining jobs and submissions, and hash-linked audit records.
- `server/src/app.ts` owns HTTP boundaries. `server/src/db.ts` owns database initialization and audit insertion. `server/src/pca10.ts` owns the deterministic mining algorithm.
- The server TypeScript build passes with `npm run build`.
- The initial server test suite has passed in single-thread Vitest mode. Keep the single-thread mode because the Windows native SQLite binding can crash during parallel worker teardown.
- Docker files and protocol/API documentation exist.
- The Rust client under `client/` is only a partial scaffold. `cargo check` passes, but the full wallet, secure Windows storage, CPU mining loop, command set, and CUDA capability are not complete.
- Do not claim PengChain V1 is complete until the server acceptance matrix and the client requirements are implemented and verified.

## Required implementation order

1. Finish and harden the server.
2. Expand behavior-focused server tests for concurrency, persistence, mining replay prevention, supply limits, adaptive difficulty, transfers, bans, authorization, and restart recovery.
3. Run the complete server test suite and fix failures.
4. Implement the Rust client against the tested server API.
5. Add cross-language PCA-10 test vectors.
6. Implement CPU mining for real. Never return a fake mining result.
7. Add a clear CUDA-unavailable error unless a genuine compatible CUDA backend exists.

## Clean-code rules

### Make the domain explicit

- Use named types and small modules for wallets, sessions, tokens, balances, transfers, mining jobs, rewards, bans, and audit events.
- Define data shapes before writing control flow.
- Keep token amounts as integer base units. Never use floating-point arithmetic for balances or rewards.
- Keep 64-bit nonces as decimal strings at HTTP boundaries and parse them with `BigInt` on the server.
- Prefer illegal-state prevention through schemas, database constraints, and narrow types.

### Keep boundaries strict

- Validate all external input at the Fastify boundary with Zod or an equivalent schema.
- Treat client-provided balances, rewards, difficulty, owner roles, timestamps, and mining results as untrusted.
- Authenticate before reading private wallet data or changing state.
- Authorize owner-only operations separately from ordinary wallet operations.
- Return safe, structured errors. Do not expose password hashes, session secrets, SQL details, or stack traces.
- Do not trust `X-Forwarded-For` alone for first-wallet initialization.

### Preserve ledger correctness

- Put every balance-changing operation and its permanent record in one SQLite transaction.
- Use idempotency keys for transfers and unique job/solution constraints for mining.
- Never delete balances or ledger history when a wallet is banned.
- Never add unrestricted minting. PENG can originate only from an accepted verified mining reward.
- Enforce custom-token max supply inside the same transaction that issues the reward.
- Keep audit records append-only in application behavior and maintain the hash link.
- Design retry behavior explicitly. Repeated requests must converge to one result.

### Keep code readable

- Use Prettier or the repository formatter before review.
- Use descriptive names instead of comments that narrate obvious code.
- Keep functions short enough that their transaction or validation scope is visible.
- Avoid compressed one-line declarations and dense chained expressions.
- Avoid `any`. If a framework boundary requires it, narrow it immediately and document the boundary.
- Prefer one responsibility per module and one reason to change per function.
- Do not add abstraction layers until a second real caller needs them.
- Keep comments for non-obvious invariants, security constraints, or protocol compatibility only.

### Test behavior

- Test through observable API behavior where practical.
- Include concurrent tests for first-wallet ownership, mining submission, and transfers.
- Test restart persistence against a real SQLite file, not only an in-memory database.
- Test invalid authentication, authorization, stale jobs, duplicate submissions, duplicate idempotency keys, negative amounts, and supply exhaustion.
- Test deterministic PCA-10 vectors with exact input bytes.
- A passing compile is not evidence that the ledger is correct.

### Documentation and delivery

- Update `docs/protocol.md` when the mining encoding or difficulty rules change.
- Update `docs/api.md` when routes, payloads, authentication, or error behavior change.
- Keep README setup instructions executable and environment-specific.
- Do not commit `node_modules`, build output, SQLite files, or Rust `target` output.
- Before reporting completion, inspect the real artifact, run the relevant build and tests, and state any unverified or unfinished requirement plainly.

## Important known gaps

The current implementation still needs adaptive difficulty, complete administrative ban/unban flows, trusted local first-wallet bootstrap enforcement, fuller rate-limit policy coverage, HTTPS deployment guidance, comprehensive restart/concurrency tests, and the complete Rust client. These are tracked as unfinished work, not as acceptable placeholders.
