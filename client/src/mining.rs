use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub use pengwallet_protocol::Job as MiningJob;

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
        && (difficulty.is_multiple_of(2) || digest[difficulty / 2] >> 4 == 0)
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

/// Parallel nonce lanes share only cancellation and counters; no nonce is tested
/// by two workers. Counters measure actual completed PCA-10 evaluations.
pub fn solve_parallel(
    job: &MiningJob,
    network: &str,
    cancelled: &AtomicBool,
    hashes: &std::sync::atomic::AtomicU64,
    threads: usize,
    intensity: u8,
) -> Result<Option<u64>> {
    use std::sync::{atomic::AtomicU64, Mutex};
    if threads == 0
        || threads > std::thread::available_parallelism()?.get()
        || ![25, 50, 100].contains(&intensity)
    {
        bail!("Unsupported CPU mining configuration");
    }
    let prefix = canonical_prefix(job, network)?;
    let expiry = UNIX_EPOCH + Duration::from_millis(job.expires_at);
    let found = AtomicBool::new(false);
    let result = Mutex::new(None);
    std::thread::scope(|scope| {
        for lane in 0..threads {
            let prefix = &prefix;
            let found = &found;
            let result = &result;
            let hashes: &AtomicU64 = hashes;
            scope.spawn(move || {
                let mut input = prefix.clone();
                input.extend_from_slice(&[0; 8]);
                let mut nonce = lane as u64;
                loop {
                    let start = std::time::Instant::now();
                    let mut completed = 0;
                    for _ in 0..128 {
                        if cancelled.load(Ordering::Relaxed)
                            || found.load(Ordering::Relaxed)
                            || SystemTime::now() >= expiry
                        {
                            break;
                        }
                        input[prefix.len()..].copy_from_slice(&nonce.to_be_bytes());
                        let valid = meets_difficulty(&pca10(&input), job.difficulty);
                        completed += 1;
                        if valid {
                            if !found.swap(true, Ordering::Relaxed) {
                                *result.lock().unwrap() = Some(nonce);
                            }
                            break;
                        }
                        match nonce.checked_add(threads as u64) {
                            Some(next) => nonce = next,
                            None => {
                                hashes.fetch_add(completed, Ordering::Relaxed);
                                return;
                            }
                        }
                    }
                    hashes.fetch_add(completed, Ordering::Relaxed);
                    if completed == 0 || found.load(Ordering::Relaxed) {
                        return;
                    }
                    if intensity < 100 {
                        let pause = start
                            .elapsed()
                            .mul_f64(100.0 / f64::from(intensity) - 1.0)
                            .min(Duration::from_millis(20));
                        std::thread::sleep(pause);
                    }
                }
            });
        }
    });
    Ok(result.into_inner().unwrap())
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

#[cfg(test)]
mod parallel_tests {
    use super::*;
    #[test]
    fn parallel_workers_find_a_real_proof_and_count_work() {
        let job = MiningJob {
            job_id: "job".into(),
            token_id: "PENG".into(),
            wallet_id: "wallet".into(),
            challenge: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
            difficulty: 3,
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 60_000,
        };
        let hashes = std::sync::atomic::AtomicU64::new(0);
        let threads = std::thread::available_parallelism().unwrap().get().min(2);
        let nonce = solve_parallel(
            &job,
            "pengchain-v1",
            &AtomicBool::new(false),
            &hashes,
            threads,
            50,
        )
        .unwrap()
        .unwrap();
        let mut input = canonical_prefix(&job, "pengchain-v1").unwrap();
        input.extend_from_slice(&nonce.to_be_bytes());
        assert!(meets_difficulty(&pca10(&input), 3));
        assert!(hashes.load(Ordering::Relaxed) > 0);
        let hashes = std::sync::atomic::AtomicU64::new(0);
        assert_eq!(
            solve_parallel(
                &job,
                "pengchain-v1",
                &AtomicBool::new(true),
                &hashes,
                threads,
                100
            )
            .unwrap(),
            None
        );
        assert_eq!(hashes.load(Ordering::Relaxed), 0);
        assert!(solve_parallel(
            &job,
            "pengchain-v1",
            &AtomicBool::new(false),
            &hashes,
            0,
            100
        )
        .is_err());
    }
}
