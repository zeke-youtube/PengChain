# PengChain V1 protocol

PengChain is a centralized server-authoritative ledger. It has no blocks, chain, peer-to-peer consensus, or public/private-key wallets.

PCA-10 hashes the exact canonical input ten times with SHA-256. Round one hashes the canonical bytes. Rounds two through ten hash the previous 32-byte digest, never its hexadecimal representation.

The canonical input is versioned length-prefixed UTF-8 fields for protocol version, network ID, job ID, token ID, wallet ID, challenge, followed by an unsigned 64-bit big-endian nonce. The server issues every field except nonce. Difficulty is the number of leading hexadecimal zero digits in the final digest.

Mining jobs expire after five minutes. A job can produce at most one reward. PENG rewards are 10 base units and are the only source of PENG supply. Custom mineable tokens use their declared reward and cannot exceed max supply.

Audit records are append-only in application behavior and hash-linked with SHA-256. A fully privileged NAS administrator can still alter SQLite, so external backups or independent checkpoints are required to reliably detect privileged tampering.
