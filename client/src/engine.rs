//! PengMiner launches this CLI mode as a child process. Authentication is passed
//! through stdin, never arguments, environment variables, or a session file.
use anyhow::{bail, Context, Result};
use pengwallet::{gpu, mining};
use pengwallet_protocol::{Event, Frame, Request, MAX_FRAME, VERSION};
use serde::de::DeserializeOwned;
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    sync::Notify,
};

fn send(event: Event) -> Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &Frame::new(event))?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
async fn read(reader: &mut BufReader<tokio::io::Stdin>) -> Result<Option<Request>> {
    let mut bytes = Vec::new();
    let count = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_FRAME {
        bail!("Engine request is too large");
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("Invalid engine request"))
}
struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}
impl Cancel {
    fn stop(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.notify.notify_one();
    }
}
struct Guard(Arc<Cancel>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.stop();
    }
}

pub async fn run() -> Result<()> {
    let mut input = BufReader::new(tokio::io::stdin());
    let request = read(&mut input)
        .await?
        .context("An engine request is required on stdin")?;
    let (version, configuration) = match request {
        Request::Inspect { version } => (version, None),
        Request::Mine {
            version,
            server,
            wallet_id,
            cookie,
            token,
            network,
            gpu,
            gpu_device,
            threads,
            intensity,
        } => (
            version,
            Some(Config {
                server,
                wallet_id,
                cookie,
                token,
                network,
                gpu,
                gpu_device,
                threads,
                intensity,
            }),
        ),
        Request::Stop => bail!("No mining process is running"),
    };
    if version != VERSION {
        bail!("Unsupported engine protocol version");
    }
    let devices = tokio::task::spawn_blocking(gpu::enumerate).await?;
    let (devices,warning)=match devices {Ok(devices)=>(devices,None),Err(_)=>(vec![],Some("OpenCL GPU discovery is unavailable. Install your vendor's driver to use GPU mining.".into()))};
    let index = configuration
        .as_ref()
        .filter(|config| config.gpu)
        .map(|config| {
            gpu::preferred_device(
                &devices,
                config
                    .gpu_device
                    .map(|index| format!("opencl:{index}"))
                    .as_deref(),
            )
        })
        .transpose()?;
    send(Event::Ready {
        cpu_count: std::thread::available_parallelism()?.get(),
        devices,
        warning,
    })?;
    let Some(mut config) = configuration else {
        return Ok(());
    };
    config.gpu_device = index;
    config.validate()?;
    let cancel = Arc::new(Cancel {
        flag: AtomicBool::new(false),
        notify: Notify::new(),
    });
    let reader_cancel = Arc::clone(&cancel);
    let reader = tokio::spawn(async move {
        let _ = read(&mut input).await;
        reader_cancel.stop();
    });
    let _guard = Guard(Arc::clone(&cancel));
    let hashes = Arc::new(AtomicU64::new(0));
    let start = Instant::now();
    let work = mine(config, Arc::clone(&cancel), Arc::clone(&hashes));
    tokio::pin!(work);
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    let mut last = Instant::now();
    let mut previous = 0;
    // The mining future cooperatively checks cancellation and joins its workers;
    // never drop a live blocking worker just because a stop frame arrived.
    let result = loop {
        tokio::select! {
            result=&mut work=>break result,
            _=ticker.tick()=>{
                let count=hashes.load(Ordering::Relaxed);let elapsed=start.elapsed().as_secs_f64();
                if let Err(error)=send(Event::Hashrate{hashes:count,elapsed,current:(count-previous) as f64/last.elapsed().as_secs_f64().max(0.001),average:count as f64/elapsed.max(0.001)}){
                    cancel.stop();let _=(&mut work).await;break Err(error);
                }
                previous=count;last=Instant::now();
            }
        }
    };
    reader.abort();
    if let Err(ref error) = result {
        let message = if error.downcast_ref::<reqwest::Error>().is_some() {
            let message =
                "Connection lost. Check the PengChain server and restart mining.".to_owned();
            let _ = send(Event::NetworkDisconnected {
                message: message.clone(),
            });
            message
        } else {
            error.to_string()
        };
        let _ = send(Event::Error { message });
    }
    send(Event::Stopped {
        hashes: hashes.load(Ordering::Relaxed),
        elapsed: start.elapsed().as_secs_f64(),
    })?;
    result
}

struct Config {
    server: String,
    wallet_id: String,
    cookie: String,
    token: String,
    network: String,
    gpu: bool,
    gpu_device: Option<usize>,
    threads: usize,
    intensity: u8,
}
impl Config {
    fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.server).context("Invalid server URL")?;
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            bail!("Engine requires a server origin without credentials or a path");
        }
        if url.scheme() == "http"
            && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        {
            bail!("Remote servers require HTTPS");
        }
        if self.threads == 0
            || self.threads > std::thread::available_parallelism()?.get()
            || ![25, 50, 100].contains(&self.intensity)
        {
            bail!("Unsupported mining configuration");
        }
        if !self.cookie.starts_with("peng_session=") || self.cookie.contains(['\r', '\n']) {
            bail!("Invalid mining session");
        }
        Ok(())
    }
}
async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    if !response.status().is_success() {
        bail!(
            "PengChain request failed (HTTP {}). Sign in again for authentication errors.",
            response.status().as_u16()
        );
    }
    response
        .json()
        .await
        .context("Unexpected PengChain response")
}
async fn mine(config: Config, cancel: Arc<Cancel>, hashes: Arc<AtomicU64>) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut gpu = if let Some(index) = config.gpu_device {
        Some(tokio::task::spawn_blocking(move || gpu::GpuMiner::new(index)).await??)
    } else {
        None
    };
    send(Event::Started {
        device_name: gpu
            .as_ref()
            .map(|g| g.name().to_owned())
            .unwrap_or_else(|| format!("CPU · {} thread(s)", config.threads)),
    })?;
    loop {
        if cancel.flag.load(Ordering::Relaxed) {
            return Ok(());
        }
        let request = client
            .get(format!("{}/blockchain/api/mining/job", config.server))
            .header("Cookie", &config.cookie)
            .query(&[("symbol", &config.token)])
            .send();
        let response =
            tokio::select! {result=request=>result?,_=cancel.notify.notified()=>return Ok(())};
        let job: mining::MiningJob = decode(response).await?;
        if job.wallet_id != config.wallet_id {
            bail!("Server returned a job for a different wallet");
        }
        send(Event::JobReceived { job: job.clone() })?;
        let id = job.job_id.clone();
        let child = Arc::clone(&cancel);
        let counter = Arc::clone(&hashes);
        let network = config.network.clone();
        let threads = config.threads;
        let intensity = config.intensity;
        let (returned, result) = tokio::task::spawn_blocking(move || {
            let result = match &gpu {
                Some(gpu) => {
                    gpu.solve_with_progress(&job, &network, &child.flag, &counter, intensity)
                }
                None => mining::solve_parallel(
                    &job,
                    &network,
                    &child.flag,
                    &counter,
                    threads,
                    intensity,
                ),
            };
            (gpu, result)
        })
        .await?;
        gpu = returned;
        if cancel.flag.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Some(nonce) = result? {
            send(Event::SolutionFound {
                job_id: id.clone(),
                nonce: nonce.to_string(),
            })?;
            // Do not cancel or retry a submitted request: read its acknowledgement
            // so an accepted reward is not lost from the graceful-stop summary.
            let response = client
                .post(format!("{}/blockchain/api/mining/submit", config.server))
                .header("Cookie", &config.cookie)
                .json(&serde_json::json!({"jobId":id,"nonce":nonce.to_string()}))
                .send()
                .await?;
            if response.status().as_u16() == 400 {
                send(Event::SubmissionRejected {
                    job_id: id,
                    nonce: nonce.to_string(),
                    message: "Server rejected the proof. Mining stopped.".into(),
                })?;
                bail!("Server rejected the mining submission");
            }
            #[derive(serde::Deserialize)]
            struct Receipt {
                accepted: bool,
                reward: u64,
                digest: String,
            }
            let receipt: Receipt = decode(response).await?;
            if !receipt.accepted {
                bail!("Server did not accept the proof");
            }
            send(Event::RewardAccepted {
                job_id: id,
                nonce: nonce.to_string(),
                token: config.token.clone(),
                reward: receipt.reward,
                digest: receipt.digest,
            })?;
        }
        if cancel.flag.load(Ordering::Relaxed) {
            return Ok(());
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_secs(3))=>(),_=cancel.notify.notified()=>return Ok(())}
    }
}
