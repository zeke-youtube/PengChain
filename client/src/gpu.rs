//! GPU-only OpenCL mining. The driver is loaded dynamically, so CPU mining
//! does not require an OpenCL SDK, driver, or GPU at build time.
use crate::mining::{canonical_prefix, meets_difficulty, pca10, MiningJob};
use anyhow::{bail, Context as _, Result};
use opencl3::{
    command_queue::CommandQueue,
    context::Context,
    device::{get_all_devices, Device, CL_DEVICE_TYPE_GPU},
    kernel::{ExecuteKernel, Kernel},
    memory::{Buffer, CL_MEM_READ_ONLY, CL_MEM_READ_WRITE},
    program::Program,
    types::{cl_device_id, CL_BLOCKING},
};
use std::{
    error::Error,
    fmt,
    process::Command,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SOURCE: &str = include_str!("kernels/pca10.cl");
const BATCH_SIZE: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareClass {
    Dedicated,
    Integrated,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareInfo {
    pub name: String,
    pub vendor: String,
    pub dedicated_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub id: String,
    pub index: usize,
    pub name: String,
    pub vendor: String,
    pub device_type: String,
    pub memory_bytes: u64,
    pub integrated: bool,
    pub available: bool,
}

#[derive(Debug)]
pub struct GpuError {
    pub code: &'static str,
    pub message: String,
    pub human_message: String,
}

impl fmt::Display for GpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for GpuError {}

fn gpu_error(
    code: &'static str,
    message: impl Into<String>,
    human_message: impl Into<String>,
) -> anyhow::Error {
    anyhow::Error::new(GpuError {
        code,
        message: message.into(),
        human_message: human_message.into(),
    })
}

pub fn classify_hardware(info: &HardwareInfo) -> HardwareClass {
    let name = info.name.to_ascii_lowercase();
    let vendor = info.vendor.to_ascii_lowercase();
    let memory = info.dedicated_memory_bytes.unwrap_or_default();
    let has_discrete_vendor = vendor.contains("nvidia")
        || vendor.contains("amd")
        || vendor.contains("advanced micro devices")
        || name.contains("geforce")
        || name.contains("radeon rx")
        || name.contains("radeon pro")
        || name.contains("arc ");
    let known_integrated_name = name.contains("uhd graphics")
        || name.contains("iris xe")
        || name.contains("iris plus")
        || name.contains("integrated graphics")
        || name == "amd radeon graphics";

    if has_discrete_vendor && memory >= 512 * 1024 * 1024 && !known_integrated_name {
        HardwareClass::Dedicated
    } else if known_integrated_name || (vendor.contains("intel") && memory < 2 * 1024 * 1024 * 1024)
    {
        HardwareClass::Integrated
    } else {
        HardwareClass::Uncertain
    }
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(windows)]
fn system_video_controllers() -> Vec<HardwareInfo> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object Name,AdapterCompatibility,AdapterRAM | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    let values = match value {
        serde_json::Value::Array(values) => values,
        value => vec![value],
    };
    values
        .into_iter()
        .filter_map(|value| {
            Some(HardwareInfo {
                name: value.get("Name")?.as_str()?.to_owned(),
                vendor: value
                    .get("AdapterCompatibility")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                dedicated_memory_bytes: value.get("AdapterRAM").and_then(serde_json::Value::as_u64),
            })
        })
        .collect()
}

#[cfg(not(windows))]
fn system_video_controllers() -> Vec<HardwareInfo> {
    Vec::new()
}

fn opencl_devices() -> Result<Vec<cl_device_id>> {
    let devices = get_all_devices(CL_DEVICE_TYPE_GPU).map_err(|error| {
        gpu_error(
            "opencl_driver_missing",
            format!("OpenCL GPU enumeration failed: {error}"),
            "OpenCL drivers are missing or unavailable. Install the latest driver from NVIDIA, AMD, or Intel, then restart PengWallet. CPU fallback is disabled.",
        )
    })?;
    if devices.is_empty() {
        return Err(gpu_error(
            "no_gpu",
            "No OpenCL GPU devices were detected",
            "No GPU was detected. Install a supported GPU driver or run without --gpu to use CPU mining.",
        ));
    }
    Ok(devices)
}

pub fn device_choices() -> Result<Vec<String>> {
    opencl_devices()?
        .into_iter()
        .map(|device| {
            Device::new(device)
                .name()
                .context("Could not read OpenCL device name")
        })
        .collect()
}

pub fn enumerate() -> Result<Vec<GpuInfo>> {
    let controllers = system_video_controllers();
    opencl_devices()?
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let device = Device::new(id);
            let name = device.name().context("Could not read OpenCL device name")?;
            let vendor = device.vendor().unwrap_or_default();
            let class = classify_opencl_device(&format!("{vendor} {name}"), &controllers);
            Ok(GpuInfo {
                id: format!("opencl:{index}"),
                index,
                name,
                vendor,
                device_type: "GPU".into(),
                memory_bytes: 0,
                integrated: class == HardwareClass::Integrated,
                available: class == HardwareClass::Dedicated,
            })
        })
        .collect()
}

pub fn preferred_device(devices: &[GpuInfo], selected: Option<&str>) -> Result<usize> {
    if let Some(selected) = selected {
        let index = selected
            .strip_prefix("opencl:")
            .context("Invalid GPU selection")?
            .parse::<usize>()?;
        let device = devices
            .iter()
            .find(|device| device.index == index)
            .context("Selected GPU was not detected")?;
        let _ = device;
        return Ok(index);
    }
    devices
        .iter()
        .find(|device| device.available && !device.integrated)
        .map(|device| device.index)
        .context("No supported dedicated GPU was detected")
}

fn classify_opencl_device(opencl_name: &str, controllers: &[HardwareInfo]) -> HardwareClass {
    let normalized_name = normalized(opencl_name);
    let exact_matches: Vec<&HardwareInfo> = controllers
        .iter()
        .filter(|controller| {
            let candidate = normalized(&controller.name);
            candidate.contains(&normalized_name) || normalized_name.contains(&candidate)
        })
        .collect();
    if exact_matches.len() == 1 {
        return classify_hardware(exact_matches[0]);
    }

    let vendor_matches: Vec<&HardwareInfo> = controllers
        .iter()
        .filter(|controller| {
            let vendor = normalized(&controller.vendor);
            !vendor.is_empty() && normalized_name.contains(&vendor)
        })
        .collect();
    if vendor_matches.len() == 1 {
        return classify_hardware(vendor_matches[0]);
    }

    HardwareClass::Uncertain
}

pub struct GpuMiner {
    kernel: Kernel,
    // OpenCL wrappers retain their dependencies; these fields also keep them
    // alive explicitly throughout all blocking dispatches.
    _program: Program,
    queue: CommandQueue,
    context: Context,
    name: String,
}

impl GpuMiner {
    pub fn new(index: usize) -> Result<Self> {
        let devices = opencl_devices()?;
        let device = devices.get(index).ok_or_else(|| {
            gpu_error(
                "gpu_index_not_found",
                format!("OpenCL GPU index {index} was not found; detected {} device(s)", devices.len()),
                "That GPU device index does not exist. Use a valid --gpu-device index. CPU fallback is disabled.",
            )
        })?;
        let opencl_device = Device::new(*device);
        let name = opencl_device
            .name()
            .context("Could not read OpenCL device name")?;
        let vendor = opencl_device.vendor().unwrap_or_default();
        let opencl_identity = format!("{vendor} {name}");
        match classify_opencl_device(&opencl_identity, &system_video_controllers()) {
            HardwareClass::Dedicated => {}
            HardwareClass::Integrated => {
                return Err(gpu_error(
                    "integrated_graphics",
                    "OpenCL device classified as integrated graphics",
                    "💀 SUCKS TO BE YOU, INTEGRATED GRAIFIX USER!\n🐧 Try --cpu instead.",
                ));
            }
            HardwareClass::Uncertain => {
                return Err(gpu_error(
                    "gpu_classification_uncertain",
                    format!("Could not reliably classify OpenCL device '{name}' as dedicated graphics"),
                    "GPU hardware classification is uncertain. Refusing to start --gpu. Use --cpu or verify your GPU driver and hardware information.",
                ));
            }
        }
        Self::from_device(*device)
    }

    fn from_device(id: cl_device_id) -> Result<Self> {
        let device = Device::new(id);
        let name = device.name().context("Could not read OpenCL device name")?;
        let context = Context::from_device(&device).context("Could not create GPU context")?;
        // The context's default device is the selected device. OpenCL 1.2 keeps
        // compatibility with vendor drivers that do not implement OpenCL 2.x.
        #[allow(deprecated)]
        let queue = CommandQueue::create_default(&context, 0)?;
        let program = Program::create_and_build_from_source(&context, SOURCE, "-cl-std=CL1.2")
            .map_err(|log| anyhow::anyhow!("Could not compile PCA-10 OpenCL kernel: {log}"))?;
        let kernel = Kernel::create(&program, "search")?;
        Ok(Self {
            kernel,
            _program: program,
            queue,
            context,
            name,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn solve(
        &self,
        job: &MiningJob,
        network: &str,
        cancelled: &AtomicBool,
    ) -> Result<Option<u64>> {
        let prefix = canonical_prefix(job, network)?;
        let expiry = UNIX_EPOCH + Duration::from_millis(job.expires_at);
        let mut base = 0u64;
        loop {
            if cancelled.load(Ordering::Relaxed) || SystemTime::now() >= expiry {
                return Ok(None);
            }
            let count = (u64::MAX - base).min((BATCH_SIZE - 1) as u64) as usize + 1;
            let found = self.search_batch(&prefix, job.difficulty, base, count)?;
            if cancelled.load(Ordering::Relaxed) || SystemTime::now() >= expiry {
                return Ok(None);
            }
            if let Some(nonce) = found {
                // Verify the small returned result on the host before sending it.
                // This detects kernel/driver errors; the nonce search runs on GPU.
                let mut input = prefix.clone();
                input.extend_from_slice(&nonce.to_be_bytes());
                if !meets_difficulty(&pca10(&input), job.difficulty) {
                    bail!("GPU returned an invalid proof of work; submission was stopped");
                }
                return Ok(Some(nonce));
            }
            match base.checked_add(count as u64) {
                Some(next) => base = next,
                None => bail!("nonce space exhausted"),
            }
        }
    }

    pub fn solve_with_progress(
        &self,
        job: &MiningJob,
        network: &str,
        cancelled: &AtomicBool,
        hashes: &AtomicU64,
        _intensity: u8,
    ) -> Result<Option<u64>> {
        let before = hashes.load(Ordering::Relaxed);
        let result = self.solve(job, network, cancelled)?;
        if result.is_some() {
            hashes.fetch_add(1, Ordering::Relaxed);
        } else if !cancelled.load(Ordering::Relaxed) {
            hashes.fetch_add(BATCH_SIZE as u64, Ordering::Relaxed);
        } else {
            hashes.fetch_max(before, Ordering::Relaxed);
        }
        Ok(result)
    }

    fn search_batch(
        &self,
        prefix: &[u8],
        difficulty: usize,
        base: u64,
        count: usize,
    ) -> Result<Option<u64>> {
        if difficulty > 64 || count == 0 || count > BATCH_SIZE {
            bail!("Invalid GPU dispatch parameters");
        }
        base.checked_add((count - 1) as u64)
            .context("GPU nonce batch overflow")?;
        let prefix_len = u32::try_from(prefix.len())?;
        let difficulty = difficulty as u32;
        // SAFETY: buffer sizes match the element types and all transfers are
        // blocking. The in-order queue completes the kernel before reading back.
        // Kernel argument order and scalar widths match pca10.cl exactly.
        unsafe {
            let mut input = Buffer::<u8>::create(
                &self.context,
                CL_MEM_READ_ONLY,
                prefix.len().max(1),
                ptr::null_mut(),
            )?;
            let mut winner =
                Buffer::<u32>::create(&self.context, CL_MEM_READ_WRITE, 1, ptr::null_mut())?;
            if !prefix.is_empty() {
                self.queue
                    .enqueue_write_buffer(&mut input, CL_BLOCKING, 0, prefix, &[])?;
            }
            self.queue
                .enqueue_write_buffer(&mut winner, CL_BLOCKING, 0, &[u32::MAX], &[])?;
            ExecuteKernel::new(&self.kernel)
                .set_arg(&input)
                .set_arg(&prefix_len)
                .set_arg(&base)
                .set_arg(&difficulty)
                .set_arg(&winner)
                .set_global_work_size(count)
                .enqueue_nd_range(&self.queue)?;
            let mut result = [u32::MAX];
            self.queue
                .enqueue_read_buffer(&winner, CL_BLOCKING, 0, &mut result, &[])?;
            if result[0] == u32::MAX {
                return Ok(None);
            }
            if result[0] as usize >= count {
                bail!("GPU returned an out-of-range nonce index");
            }
            Ok(Some(base + u64::from(result[0])))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencl3::device::CL_DEVICE_TYPE_CPU;

    #[test]
    fn classifies_dedicated_gpu_from_hardware_metadata() {
        let info = HardwareInfo {
            name: "NVIDIA GeForce RTX 4070".into(),
            vendor: "NVIDIA".into(),
            dedicated_memory_bytes: Some(12 * 1024 * 1024 * 1024),
        };
        assert_eq!(classify_hardware(&info), HardwareClass::Dedicated);
    }

    #[test]
    fn classifies_integrated_graphics_from_hardware_metadata() {
        let info = HardwareInfo {
            name: "Intel(R) UHD Graphics 770".into(),
            vendor: "Intel Corporation".into(),
            dedicated_memory_bytes: Some(128 * 1024 * 1024),
        };
        assert_eq!(classify_hardware(&info), HardwareClass::Integrated);
    }

    #[test]
    fn refuses_uncertain_hardware_metadata() {
        let info = HardwareInfo {
            name: "Generic OpenCL GPU".into(),
            vendor: "Unknown".into(),
            dedicated_memory_bytes: None,
        };
        assert_eq!(classify_hardware(&info), HardwareClass::Uncertain);
    }

    #[test]
    fn distinguishes_no_matching_hardware_record() {
        assert_eq!(
            classify_opencl_device(
                "OpenCL GPU",
                &[HardwareInfo {
                    name: "Different GPU".into(),
                    vendor: "Unknown".into(),
                    dedicated_memory_bytes: None,
                }]
            ),
            HardwareClass::Uncertain
        );
    }

    #[test]
    fn matches_opencl_name_to_unique_vendor_when_model_names_differ() {
        let controllers = vec![
            HardwareInfo {
                name: "AMD Radeon(TM) Graphics".into(),
                vendor: "Advanced Micro Devices, Inc.".into(),
                dedicated_memory_bytes: Some(2 * 1024 * 1024 * 1024),
            },
            HardwareInfo {
                name: "NVIDIA GeForce RTX 5070 Ti".into(),
                vendor: "NVIDIA".into(),
                dedicated_memory_bytes: Some(4 * 1024 * 1024 * 1024),
            },
        ];
        assert_eq!(
            classify_opencl_device("NVIDIA RTX 5070 Ti OpenCL", &controllers),
            HardwareClass::Dedicated
        );
    }

    #[test]
    #[ignore = "requires an OpenCL device; see README for explicit GPU/software conformance runs"]
    fn opencl_kernel_matches_rust_sha256() -> Result<()> {
        // CPU OpenCL is permitted ONLY inside this explicit test. The user-facing
        // constructor always selects CL_DEVICE_TYPE_GPU, with no fallback.
        let kind = match std::env::var("PENGCHAIN_TEST_OPENCL_DEVICE").as_deref() {
            Ok("cpu") => CL_DEVICE_TYPE_CPU,
            Ok("gpu") | Err(_) => CL_DEVICE_TYPE_GPU,
            Ok(value) => bail!("Unknown conformance device type: {value}"),
        };
        let devices = get_all_devices(kind)?;
        let miner = GpuMiner::from_device(*devices.first().context("No OpenCL test device")?)?;
        eprintln!("OpenCL conformance device: {}", miner.name());
        let digest_kernel = Kernel::create(&miner._program, "digests")?;
        for length in [0, 1, 47, 48, 55, 56, 63, 64, 65, 119, 120, 128, 240] {
            let prefix: Vec<u8> = (0..length).map(|i| (i % 256) as u8).collect();
            for (base, count) in [(0u64, 8usize), (u64::MAX - 1, 2)] {
                let prefix_len = length as u32;
                // SAFETY: exact kernel argument types, sized buffers and blocking
                // transfers as in search_batch; count also bounds the output.
                let words = unsafe {
                    let mut input = Buffer::<u8>::create(
                        &miner.context,
                        CL_MEM_READ_ONLY,
                        length.max(1),
                        ptr::null_mut(),
                    )?;
                    let output = Buffer::<u32>::create(
                        &miner.context,
                        CL_MEM_READ_WRITE,
                        count * 8,
                        ptr::null_mut(),
                    )?;
                    if !prefix.is_empty() {
                        miner.queue.enqueue_write_buffer(
                            &mut input,
                            CL_BLOCKING,
                            0,
                            &prefix,
                            &[],
                        )?;
                    }
                    ExecuteKernel::new(&digest_kernel)
                        .set_arg(&input)
                        .set_arg(&prefix_len)
                        .set_arg(&base)
                        .set_arg(&output)
                        .set_global_work_size(count)
                        .enqueue_nd_range(&miner.queue)?;
                    let mut words = vec![0u32; count * 8];
                    miner
                        .queue
                        .enqueue_read_buffer(&output, CL_BLOCKING, 0, &mut words, &[])?;
                    words
                };
                for index in 0..count {
                    let mut input = prefix.clone();
                    input.extend_from_slice(&(base + index as u64).to_be_bytes());
                    let actual: Vec<u8> = words[index * 8..index * 8 + 8]
                        .iter()
                        .flat_map(|word| word.to_be_bytes())
                        .collect();
                    assert_eq!(
                        actual,
                        pca10(&input),
                        "prefix length {length}, nonce {}",
                        base + index as u64
                    );
                }
            }
        }
        let job = MiningJob {
            job_id: "job".into(),
            token_id: "PENG".into(),
            wallet_id: "wallet".into(),
            challenge: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
            difficulty: 3,
            expires_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64 + 60_000,
        };
        let prefix = canonical_prefix(&job, "pengchain-v1")?;
        for difficulty in [0, 1, 3, 64] {
            let expected = (0..BATCH_SIZE as u64).find(|nonce| {
                let mut input = prefix.clone();
                input.extend_from_slice(&nonce.to_be_bytes());
                meets_difficulty(&pca10(&input), difficulty)
            });
            assert_eq!(
                miner.search_batch(&prefix, difficulty, 0, BATCH_SIZE)?,
                expected
            );
        }
        assert_eq!(
            miner.solve(&job, "pengchain-v1", &AtomicBool::new(false))?,
            Some(6306)
        );
        assert_eq!(
            miner.solve(&job, "pengchain-v1", &AtomicBool::new(true))?,
            None
        );
        let expired = MiningJob {
            expires_at: 0,
            ..job
        };
        assert_eq!(
            miner.solve(&expired, "pengchain-v1", &AtomicBool::new(false))?,
            None
        );
        assert!(miner.search_batch(&prefix, 1, u64::MAX, 2).is_err());
        assert_eq!(miner.search_batch(&prefix, 0, u64::MAX, 1)?, Some(u64::MAX));
        Ok(())
    }
}
