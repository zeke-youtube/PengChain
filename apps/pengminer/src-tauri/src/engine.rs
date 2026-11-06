use crate::{
    api::{Api, ApiError, AuditRecord, Balance, CreateTokenRequest, Session, Token},
    storage::{Files, SessionStore, Settings},
};
use anyhow::{bail, Context, Result};
use pengwallet::{
    gpu::{self, GpuInfo, GpuMiner},
    mining::{self, MiningJob},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex as AsyncMutex, Notify},
    task::JoinHandle,
};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Solution {
    pub time: u64,
    pub wallet_id: String,
    pub server: String,
    pub token: String,
    pub job_id: String,
    pub nonce: String,
    pub accepted: bool,
    pub reward: u64,
}
#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    pub time: u64,
    pub kind: String,
    pub message: String,
}
#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub running: bool,
    pub connected: bool,
    pub wallet_id: Option<String>,
    pub persistent: bool,
    pub settings: Settings,
    pub current_hashrate: f64,
    pub average_hashrate: f64,
    pub hashes: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub earnings: u64,
    pub difficulty: Option<usize>,
    pub uptime: f64,
    pub device_name: String,
    pub job: Option<MiningJob>,
    pub balances: Vec<Balance>,
    pub tokens: Vec<Token>,
    pub history: Vec<Solution>,
    pub logs: Vec<Log>,
    pub audit_logs: Vec<AuditRecord>,
    pub error: Option<String>,
    pub cpu_count: usize,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiningEvent {
    pub kind: String,
    pub snapshot: Snapshot,
}
pub type Listener = Arc<dyn Fn(MiningEvent) + Send + Sync>;

struct State {
    snapshot: Snapshot,
    session: Option<Session>,
}
struct Cancellation {
    flag: AtomicBool,
    notify: Notify,
}
impl Cancellation {
    fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }
    fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.notify.notify_one();
    }
}
struct StopWorker(Arc<Cancellation>);
impl Drop for StopWorker {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct Task {
    cancel: Arc<Cancellation>,
    handle: JoinHandle<()>,
}

struct ActiveWorker(Arc<AtomicUsize>);
impl ActiveWorker {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(count))
    }
}
impl Drop for ActiveWorker {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Engine {
    hashes: Arc<AtomicU64>,
    workers: Arc<AtomicUsize>,
    state: Mutex<State>,
    commands: AsyncMutex<()>,
    task: AsyncMutex<Option<Task>>,
    store: Arc<dyn SessionStore>,
    files: Files,
    listener: Listener,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn public_error(error: &anyhow::Error) -> String {
    if let Some(error) = error.downcast_ref::<ApiError>() {
        return error.message.clone();
    }
    if error.downcast_ref::<reqwest::Error>().is_some() {
        return "Connection lost. Check the server address and network, then restart mining."
            .into();
    }
    error
        .to_string()
        .chars()
        .filter(|c| !c.is_control())
        .take(500)
        .collect()
}

impl Engine {
    pub async fn create_token(self: &Arc<Self>, request: CreateTokenRequest) -> Result<Snapshot> {
        let _guard = self.commands.lock().await;
        let session = self
            .state
            .lock()
            .unwrap()
            .session
            .clone()
            .context("Sign in before creating a token")?;
        Api::new(&session.server)?
            .create_token(&session, &request)
            .await?;
        self.refresh().await
    }
    pub fn new(
        store: Arc<dyn SessionStore>,
        files: Files,
        listener: Listener,
    ) -> Result<Arc<Self>> {
        let settings = files.settings()?;
        Ok(Arc::new(Self {
            hashes: Arc::new(AtomicU64::new(0)),
            workers: Arc::new(AtomicUsize::new(0)),
            state: Mutex::new(State {
                session: None,
                snapshot: Snapshot {
                    running: false,
                    connected: false,
                    wallet_id: None,
                    persistent: false,
                    settings,
                    current_hashrate: 0.0,
                    average_hashrate: 0.0,
                    hashes: 0,
                    accepted: 0,
                    rejected: 0,
                    earnings: 0,
                    difficulty: None,
                    uptime: 0.0,
                    device_name: "Not selected".into(),
                    job: None,
                    balances: vec![],
                    tokens: vec![],
                    history: vec![],
                    logs: vec![],
                    audit_logs: vec![],
                    error: None,
                    cpu_count: std::thread::available_parallelism()?.get(),
                },
            }),
            commands: AsyncMutex::new(()),
            task: AsyncMutex::new(None),
            store,
            files,
            listener,
        }))
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().snapshot.clone()
    }
    fn emit(&self, kind: &str, message: Option<&str>) {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            if let Some(message) = message {
                state.snapshot.logs.push(Log {
                    time: now(),
                    kind: kind.into(),
                    message: message.into(),
                });
                if state.snapshot.logs.len() > 150 {
                    state.snapshot.logs.remove(0);
                }
            }
            state.snapshot.clone()
        };
        (self.listener)(MiningEvent {
            kind: kind.into(),
            snapshot,
        });
    }
    fn history(&self, session: &Session) -> Vec<Solution> {
        std::fs::read(self.files.directory.join("rewards.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Solution>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.server == session.server && s.wallet_id == session.wallet_id)
            .collect()
    }
    pub async fn initialize(self: &Arc<Self>) -> Result<Snapshot> {
        let guard = self.commands.lock().await;
        let server = self.snapshot().settings.server;
        let store = Arc::clone(&self.store);
        let saved = tokio::task::spawn_blocking(move || store.load(&server)).await?;
        match saved {
            Ok(Some(session)) => {
                if let Ok(balances) = Api::new(&session.server)?.balances(&session).await {
                    let audit_logs = Api::new(&session.server)?
                        .logs(&session, None)
                        .await
                        .unwrap_or_default();
                    let mut state = self.state.lock().unwrap();
                    state.snapshot.history = self.history(&session);
                    state.snapshot.wallet_id = Some(session.wallet_id.clone());
                    state.snapshot.persistent = true;
                    state.snapshot.balances = balances;
                    state.snapshot.audit_logs = audit_logs;
                    state.snapshot.connected = true;
                    state.session = Some(session);
                } else { self.emit("session_expired",Some("Saved session could not be validated. Sign in to continue.")); }
            }
            Ok(None) => (),
            Err(_) => self.emit("storage_unavailable",Some("OS credential storage is unavailable. Sign-in will last for this application session only.")),
        }
        drop(guard);
        let _ = self.refresh().await;
        if self.snapshot().settings.start_on_launch && self.snapshot().wallet_id.is_some() {
            if let Err(error) = self.start().await {
                let message = public_error(&error);
                self.state.lock().unwrap().snapshot.error = Some(message.clone());
                self.emit("mining_error", Some(&message));
            }
        }
        Ok(self.snapshot())
    }
    pub async fn login(self: &Arc<Self>, wallet: String, password: String) -> Result<Snapshot> {
        let _guard = self.commands.lock().await;
        if self.snapshot().running {
            bail!("Stop mining before changing wallets");
        }
        let api = Api::new(&self.snapshot().settings.server)?;
        let session = api.login(wallet.trim(), &password).await?;
        drop(password);
        let balances = api.balances(&session).await?;
        let audit_logs = api.logs(&session, None).await.unwrap_or_default();
        let saved = session.clone();
        let store = Arc::clone(&self.store);
        let persistent = tokio::task::spawn_blocking(move || store.save(&saved))
            .await?
            .is_ok();
        {
            let mut state = self.state.lock().unwrap();
            state.snapshot.wallet_id = Some(session.wallet_id.clone());
            state.snapshot.persistent = persistent;
            state.snapshot.connected = true;
            state.snapshot.error = None;
            state.snapshot.balances = balances;
            state.snapshot.audit_logs = audit_logs;
            state.snapshot.history = self.history(&session);
            state.session = Some(session);
        }
        self.emit("signed_in",Some(if persistent { "Signed in. Session saved in OS credential storage." } else { "Signed in for this session. OS credential storage is unavailable; no session was written to disk." }));
        Ok(self.snapshot())
    }
    pub async fn logout(self: &Arc<Self>) -> Result<Snapshot> {
        let _guard = self.commands.lock().await;
        self.stop_inner().await?;
        let session = self.state.lock().unwrap().session.clone();
        if let Some(session) = session {
            let api = Api::new(&session.server)?;
            let remote = api.logout(&session).await;
            let store = Arc::clone(&self.store);
            let server = session.server;
            let cleared = tokio::task::spawn_blocking(move || store.clear(&server)).await?;
            {
                let mut state = self.state.lock().unwrap();
                state.session = None;
                state.snapshot.wallet_id = None;
                state.snapshot.persistent = false;
                state.snapshot.balances.clear();
                state.snapshot.audit_logs.clear();
                state.snapshot.history.clear();
            }
            if cleared.is_err() {
                self.emit("storage_error",Some("Could not remove the OS credential. Remove the PengMiner entry from your credential manager before closing."));
            }
            if remote.is_err() {
                self.emit(
                    "network_disconnected",
                    Some(
                        "Local sign-out complete; the server could not confirm session revocation.",
                    ),
                );
            }
        }
        self.emit("signed_out", Some("Signed out."));
        Ok(self.snapshot())
    }
    pub async fn save_settings(self: &Arc<Self>, mut settings: Settings) -> Result<Snapshot> {
        let _guard = self.commands.lock().await;
        if self.snapshot().running {
            bail!("Stop mining before changing settings");
        }
        settings.validate()?;
        self.files.write("settings.json", &settings)?;
        let mut state = self.state.lock().unwrap();
        if state.snapshot.settings.server != settings.server {
            state.session = None;
            state.snapshot.wallet_id = None;
            state.snapshot.balances.clear();
            state.snapshot.history.clear();
            state.snapshot.persistent = false;
            state.snapshot.connected = false;
        }
        state.snapshot.settings = settings;
        drop(state);
        self.emit("settings_saved", Some("Settings saved."));
        Ok(self.snapshot())
    }
    pub async fn refresh(self: &Arc<Self>) -> Result<Snapshot> {
        let settings = self.snapshot().settings;
        let api = Api::new(&settings.server)?;
        let session = self.state.lock().unwrap().session.clone();
        let result = async {
            api.health().await?;
            let tokens = api.tokens().await?;
            let balances = if let Some(ref session) = session {
                Some(api.balances(session).await?)
            } else {
                None
            };
            let audit_logs = if let Some(ref session) = session {
                api.logs(session, None).await.unwrap_or_default()
            } else {
                vec![]
            };
            Ok::<_, anyhow::Error>((tokens, balances, audit_logs))
        }
        .await;
        match result {
            Ok((tokens, balances, audit_logs)) => {
                let mut state = self.state.lock().unwrap();
                // A late refresh may never apply data to a newly selected server/wallet.
                if state.snapshot.settings.server != settings.server
                    || state.snapshot.wallet_id != session.as_ref().map(|s| s.wallet_id.clone())
                {
                    return Ok(state.snapshot.clone());
                }
                state.snapshot.connected = true;
                state.snapshot.error = None;
                state.snapshot.tokens = tokens;
                if let Some(balances) = balances {
                    state.snapshot.balances = balances;
                }
                state.snapshot.audit_logs = audit_logs;
                Ok(state.snapshot.clone())
            }
            Err(error) => {
                let relevant = {
                    let mut state = self.state.lock().unwrap();
                    let relevant = state.snapshot.settings.server == settings.server
                        && state.snapshot.wallet_id
                            == session.as_ref().map(|s| s.wallet_id.clone());
                    if relevant {
                        state.snapshot.connected = false;
                        state.snapshot.error = Some(public_error(&error));
                    }
                    relevant
                };
                if relevant {
                    if let Some(task) = self.task.lock().await.as_ref() {
                        task.cancel.cancel();
                    }
                    self.emit(
                        "network_disconnected",
                        Some(
                            "Server connection or authentication failed. Mining has been stopped.",
                        ),
                    );
                }
                Err(error)
            }
        }
    }
    pub async fn devices() -> Result<(Vec<GpuInfo>, Option<String>)> {
        match tokio::task::spawn_blocking(gpu::enumerate).await? {
            Ok(devices)=>Ok((devices,None)),
            Err(_)=>Ok((vec![],Some("OpenCL is unavailable. Install a supported vendor driver to enable GPU mining.".into()))),
        }
    }
    pub async fn start(self: &Arc<Self>) -> Result<Snapshot> {
        let _guard = self.commands.lock().await;
        if self.snapshot().running {
            bail!("A mining session is already running");
        }
        self.stop_inner().await?;
        let mut settings = self.snapshot().settings;
        settings.validate()?;
        let session = self
            .state
            .lock()
            .unwrap()
            .session
            .clone()
            .context("Sign in before starting mining")?;
        let api = Api::new(&settings.server)?;
        api.balances(&session).await?;
        let gpu = if settings.device == "cpu" {
            None
        } else {
            let selected = if settings.device == "gpu:auto" {
                None
            } else {
                Some(settings.device.clone())
            };
            let devices = tokio::task::spawn_blocking(gpu::enumerate).await??;
            Some(gpu::preferred_device(&devices, selected.as_deref())?)
        };
        {
            let mut state = self.state.lock().unwrap();
            let snapshot = &mut state.snapshot;
            snapshot.running = true;
            snapshot.error = None;
            snapshot.connected = true;
            snapshot.accepted = 0;
            snapshot.rejected = 0;
            snapshot.earnings = 0;
            snapshot.hashes = 0;
            snapshot.current_hashrate = 0.0;
            snapshot.average_hashrate = 0.0;
            snapshot.uptime = 0.0;
            snapshot.job = None;
            snapshot.difficulty = None;
        }
        self.hashes.store(0, Ordering::Relaxed);
        let cancel = Arc::new(Cancellation::new());
        let child = Arc::clone(&cancel);
        let engine = Arc::clone(self);
        let handle = tokio::spawn(async move {
            engine.run(api, session, settings, gpu, child).await;
        });
        *self.task.lock().await = Some(Task { cancel, handle });
        self.emit(
            "mining_started",
            Some("Mining started. Rewards are counted only after server acceptance."),
        );
        Ok(self.snapshot())
    }
    pub async fn stop(self: &Arc<Self>) -> Result<Snapshot> {
        let _guard = self.commands.lock().await;
        self.stop_inner().await?;
        let _ = self.refresh().await;
        Ok(self.snapshot())
    }
    async fn stop_inner(&self) -> Result<()> {
        if let Some(task) = self.task.lock().await.take() {
            task.cancel.cancel();
            task.handle
                .await
                .context("Mining worker stopped unexpectedly")?;
        }
        while self.workers.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        {
            let mut state = self.state.lock().unwrap();
            state.snapshot.hashes = self.hashes.load(Ordering::Relaxed);
            state.snapshot.average_hashrate =
                state.snapshot.hashes as f64 / state.snapshot.uptime.max(0.001);
        }
        Ok(())
    }
    async fn run(
        self: Arc<Self>,
        api: Api,
        session: Session,
        settings: Settings,
        gpu: Option<usize>,
        cancel: Arc<Cancellation>,
    ) {
        let hashes = Arc::clone(&self.hashes);
        let started = Instant::now();
        let guard = StopWorker(Arc::clone(&cancel));
        let result = {
            let work = self.mine(
                api,
                session,
                settings,
                gpu,
                Arc::clone(&cancel),
                Arc::clone(&hashes),
            );
            tokio::pin!(work);
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            let mut last = Instant::now();
            let mut previous = 0;
            loop {
                tokio::select! {
                    biased;
                    _=cancel.notify.notified()=>break Ok(()),
                    result=&mut work=>break result,
                    _=interval.tick()=> {
                        let count=hashes.load(Ordering::Relaxed);let elapsed=last.elapsed().as_secs_f64();
                        {let mut state=self.state.lock().unwrap();let snapshot=&mut state.snapshot;
                         snapshot.hashes=count;snapshot.uptime=started.elapsed().as_secs_f64();
                         snapshot.current_hashrate=(count-previous) as f64 / elapsed.max(0.001);
                         snapshot.average_hashrate=count as f64 / snapshot.uptime.max(0.001);}
                        last=Instant::now();previous=count;self.emit("hashrate_updated",None);
                    }
                }
            }
        };
        drop(guard);
        {
            let mut state = self.state.lock().unwrap();
            let snapshot = &mut state.snapshot;
            snapshot.running = false;
            snapshot.current_hashrate = 0.0;
            snapshot.hashes = hashes.load(Ordering::Relaxed);
            snapshot.uptime = started.elapsed().as_secs_f64();
            snapshot.average_hashrate = snapshot.hashes as f64 / snapshot.uptime.max(0.001);
            snapshot.job = None;
            if let Err(ref error) = result {
                snapshot.error = Some(public_error(error));
            }
        }
        if let Err(error) = result {
            let message = public_error(&error);
            if error.downcast_ref::<reqwest::Error>().is_some() {
                self.state.lock().unwrap().snapshot.connected = false;
                self.emit("network_disconnected",Some("Network disconnected. Mining stopped; no submission will be retried automatically."));
            }
            self.emit("mining_error", Some(&message));
        }
        self.emit(
            "mining_stopped",
            Some(
                "Mining stopped. Any in-flight submission can be checked using the wallet balance.",
            ),
        );
    }
    async fn mine(
        &self,
        api: Api,
        session: Session,
        settings: Settings,
        index: Option<usize>,
        cancel: Arc<Cancellation>,
        hashes: Arc<AtomicU64>,
    ) -> Result<()> {
        let mut gpu = if let Some(index) = index {
            Some({
                let active = ActiveWorker::new(&self.workers);
                tokio::task::spawn_blocking(move || {
                    let _active = active;
                    GpuMiner::new(index)
                })
                .await??
            })
        } else {
            None
        };
        self.state.lock().unwrap().snapshot.device_name = gpu
            .as_ref()
            .map(|g| g.name().to_owned())
            .unwrap_or_else(|| format!("CPU · {} thread(s)", settings.cpu_threads));
        loop {
            if cancel.flag.load(Ordering::Relaxed) {
                return Ok(());
            }
            let job = api.job(&session, &settings.token).await?;
            {
                let mut state = self.state.lock().unwrap();
                state.snapshot.difficulty = Some(job.difficulty);
                state.snapshot.job = Some(job.clone());
            }
            self.emit("mining_job_received", Some("Fresh mining job received."));
            let child = Arc::clone(&cancel);
            let counter = Arc::clone(&hashes);
            let network = settings.network.clone();
            let threads = settings.cpu_threads;
            let intensity = settings.intensity;
            let id = job.job_id.clone();
            let active = ActiveWorker::new(&self.workers);
            let (returned, result) = tokio::task::spawn_blocking(move || {
                let _active = active;
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
            if let Some(nonce) = result? {
                if cancel.flag.load(Ordering::Relaxed) {
                    return Ok(());
                }
                self.emit(
                    "solution_found",
                    Some("Proof found. Awaiting server verification."),
                );
                // Exactly one submit per job. A lost response is not retried.
                let response = api.submit(&session, &id, nonce).await;
                let receipt = match response {
                    Ok(receipt) if receipt.accepted => receipt,
                    Ok(_) => {
                        self.reject(&session, &settings.token, &id, nonce);
                        bail!("Server rejected the solution");
                    }
                    Err(error) => {
                        if error
                            .downcast_ref::<ApiError>()
                            .is_some_and(|e| e.status == 400)
                        {
                            self.reject(&session, &settings.token, &id, nonce);
                        }
                        return Err(error);
                    }
                };
                let history = {
                    let mut state = self.state.lock().unwrap();
                    let snapshot = &mut state.snapshot;
                    snapshot.accepted += 1;
                    snapshot.earnings = snapshot
                        .earnings
                        .checked_add(receipt.reward)
                        .context("Earnings overflow")?;
                    snapshot.history.push(Solution {
                        time: now(),
                        server: session.server.clone(),
                        wallet_id: session.wallet_id.clone(),
                        token: settings.token.clone(),
                        job_id: id,
                        nonce: nonce.to_string(),
                        accepted: true,
                        reward: receipt.reward,
                    });
                    if snapshot.history.len() > 200 {
                        snapshot.history.remove(0);
                    }
                    snapshot.history.clone()
                };
                let mut saved: Vec<Solution> =
                    std::fs::read(self.files.directory.join("rewards.json"))
                        .ok()
                        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                        .unwrap_or_default();
                saved.retain(|row| {
                    row.wallet_id != session.wallet_id || row.server != session.server
                });
                saved.extend(history);
                if saved.len() > 1000 {
                    saved.drain(..saved.len() - 1000);
                }
                if self.files.write("rewards.json", &saved).is_err() {
                    self.emit("storage_error",Some("Reward accepted, but local history could not be saved. Server balance remains authoritative."));
                }
                self.emit(
                    "reward_accepted",
                    Some("Verified reward accepted by the server."),
                );
                let balances = api.balances(&session).await?;
                self.state.lock().unwrap().snapshot.balances = balances;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
    fn reject(&self, session: &Session, token: &str, job: &str, nonce: u64) {
        {
            let mut state = self.state.lock().unwrap();
            state.snapshot.rejected += 1;
            state.snapshot.history.push(Solution {
                time: now(),
                server: session.server.clone(),
                wallet_id: session.wallet_id.clone(),
                token: token.into(),
                job_id: job.into(),
                nonce: nonce.to_string(),
                accepted: false,
                reward: 0,
            });
            if state.snapshot.history.len() > 200 {
                state.snapshot.history.remove(0);
            }
        }
        self.emit(
            "submission_rejected",
            Some("Server rejected the submission. Mining stopped."),
        );
    }
}
