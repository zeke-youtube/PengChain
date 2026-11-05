use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiningJob {
    pub job_id: String,
    pub token_id: String,
    pub wallet_id: String,
    pub challenge: String,
    pub difficulty: usize,
    pub expires_at: u64,
}

pub(crate) fn canonical_prefix(job: &MiningJob, network: &str) -> Result<Vec<u8>> {
    let challenge = URL_SAFE_NO_PAD
        .decode(&job.challenge)
        .context("invalid job challenge")?;
    if challenge.len() != 32 || job.difficulty > 64 {
        bail!("invalid mining job challenge length or difficulty");
    }
    let mut input = Vec::new();
    for field in [
        &[1u8][..],
        network.as_bytes(),
        job.job_id.as_bytes(),
        job.token_id.as_bytes(),
        job.wallet_id.as_bytes(),
        &challenge,
    ] {
        input.extend_from_slice(&u32::try_from(field.len())?.to_be_bytes());
        input.extend_from_slice(field);
    }
    Ok(input)
}

pub(crate) fn pca10(input: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::digest(input);
    for _ in 1..10 {
        digest = Sha256::digest(digest);
    }
    digest.into()
}

pub(crate) fn meets_difficulty(digest: &[u8; 32], difficulty: usize) -> bool {
    difficulty <= 64
        && digest[..difficulty / 2].iter().all(|byte| *byte == 0)
        && (difficulty % 2 == 0 || digest[difficulty / 2] >> 4 == 0)
}

pub fn solve(job: &MiningJob, network: &str, cancelled: &AtomicBool) -> Result<Option<u64>> {
    let mut input = canonical_prefix(job, network)?;
    let prefix_len = input.len();
    input.extend_from_slice(&[0; 8]);
    let expiry = UNIX_EPOCH + Duration::from_millis(job.expires_at);
    for nonce in 0..=u64::MAX {
        if cancelled.load(Ordering::Relaxed) || SystemTime::now() >= expiry {
            return Ok(None);
        }
        input[prefix_len..].copy_from_slice(&nonce.to_be_bytes());
        if meets_difficulty(&pca10(&input), job.difficulty) {
            return Ok(Some(nonce));
        }
    }
    bail!("nonce space exhausted")
}

pub enum Miner {
    Cpu,
    Gpu(Box<crate::gpu::GpuMiner>),
}
impl Miner {
    pub fn new(gpu: bool, index: usize) -> Result<Self> {
        if gpu {
            Ok(Self::Gpu(Box::new(crate::gpu::GpuMiner::new(index)?)))
        } else {
            Ok(Self::Cpu)
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Cpu => "CPU",
            Self::Gpu(gpu) => gpu.name(),
        }
    }
    pub fn solve(
        &self,
        job: &MiningJob,
        network: &str,
        cancelled: &AtomicBool,
    ) -> Result<Option<u64>> {
        match self {
            Self::Cpu => solve(job, network, cancelled),
            Self::Gpu(gpu) => gpu.solve(job, network, cancelled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> MiningJob {
        MiningJob {
            job_id: "job".into(),
            token_id: "PENG".into(),
            wallet_id: "wallet".into(),
            challenge: URL_SAFE_NO_PAD.encode([0; 32]),
            difficulty: 3,
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 60_000,
        }
    }

    #[test]
    fn solves_canonical_cross_language_vector() {
        // Independently calculated with Python hashlib and big-endian struct packing.
        assert_eq!(
            solve(&job(), "pengchain-v1", &AtomicBool::new(false)).unwrap(),
            Some(6306)
        );
    }

    #[test]
    fn rejects_expired_and_malformed_jobs() {
        let mut job = job();
        job.expires_at = 0;
        assert_eq!(
            solve(&job, "pengchain-v1", &AtomicBool::new(false)).unwrap(),
            None
        );
        job.challenge = "invalid!".into();
        assert!(canonical_prefix(&job, "pengchain-v1").is_err());
        job.challenge = URL_SAFE_NO_PAD.encode([0; 32]);
        job.difficulty = 65;
        assert!(canonical_prefix(&job, "pengchain-v1").is_err());
    }

    #[test]
    fn stops_cpu_search_when_cancelled() {
        assert_eq!(
            solve(&job(), "pengchain-v1", &AtomicBool::new(true)).unwrap(),
            None
        );
    }

    #[test]
    fn raw_digest_vector() {
        assert_eq!(
            pca10(b"pengchain"),
            [
                171, 43, 150, 27, 159, 162, 146, 163, 157, 74, 113, 102, 138, 198, 68, 147, 4, 166,
                13, 104, 6, 21, 71, 31, 211, 82, 234, 17, 18, 38, 189, 219
            ]
        );
    }

    #[test]
    fn difficulty_counts_hex_digits() {
        let mut digest = [0; 32];
        digest[1] = 0x0f;
        assert!(meets_difficulty(&digest, 3));
        assert!(!meets_difficulty(&digest, 4));
        assert!(!meets_difficulty(&digest, 65));
        assert!(meets_difficulty(&[0; 32], 64));
    }
}
