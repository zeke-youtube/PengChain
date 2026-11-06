//! Versioned JSON-lines IPC types. No mining code or driver is linked here.
use serde::{Deserialize, Serialize};
pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 65_536;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub job_id: String,
    pub token_id: String,
    pub wallet_id: String,
    pub challenge: String,
    pub difficulty: usize,
    pub expires_at: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub index: usize,
    pub name: String,
    pub vendor: String,
    pub device_type: String,
    pub memory_bytes: u64,
    pub integrated: bool,
    pub available: bool,
}
pub fn preferred_gpu(devices: &[Device], selected: Option<&str>) -> Option<usize> {
    devices
        .iter()
        .find(|device| {
            device.available
                && match selected {
                    Some(id) => device.id == id,
                    None => !device.integrated,
                }
        })
        .map(|device| device.index)
}
// Deliberately no Debug implementation: this request contains a session secret.
#[derive(Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    Inspect {
        version: u32,
    },
    Mine {
        version: u32,
        server: String,
        wallet_id: String,
        cookie: String,
        token: String,
        network: String,
        gpu: bool,
        gpu_device: Option<usize>,
        threads: usize,
        intensity: u8,
    },
    Stop,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Ready {
        cpu_count: usize,
        devices: Vec<Device>,
        warning: Option<String>,
    },
    Started {
        device_name: String,
    },
    JobReceived {
        job: Job,
    },
    Hashrate {
        hashes: u64,
        elapsed: f64,
        current: f64,
        average: f64,
    },
    SolutionFound {
        job_id: String,
        nonce: String,
    },
    RewardAccepted {
        job_id: String,
        nonce: String,
        token: String,
        reward: u64,
        digest: String,
    },
    SubmissionRejected {
        job_id: String,
        nonce: String,
        message: String,
    },
    NetworkDisconnected {
        message: String,
    },
    Error {
        message: String,
    },
    Stopped {
        hashes: u64,
        elapsed: f64,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub event: Event,
}
impl Frame {
    pub fn new(event: Event) -> Self {
        Self {
            protocol_version: VERSION,
            event,
        }
    }
}
