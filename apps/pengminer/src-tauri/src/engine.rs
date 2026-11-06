use crate::{
    api::{Api, ApiError, Balance, Session, Token},
    storage::{Files, SessionStore, Settings},
};
use anyhow::{bail, Context, Result};
use pengwallet_protocol::{
    Device as GpuInfo, Event as CliEvent, Job as MiningJob, Request, VERSION,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
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
struct Task {
    cancel: Arc<Cancellation>,
    handle: JoinHandle<()>,
}

pub struct Engine {
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
    pub fn new(
        store: Arc<dyn SessionStore>,
        files: Files,
        listener: Listener,
    ) -> Result<Arc<Self>> {
        let settings = files.settings()?;
        Ok(Arc::new(Self {
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
                    let mut state=self.state.lock().unwrap();
                    state.snapshot.history=self.history(&session);
                    state.snapshot.wallet_id=Some(session.wallet_id.clone());state.snapshot.persistent=true;
                    state.snapshot.balances=balances;state.snapshot.connected=true;state.session=Some(session);
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
            Ok::<_, anyhow::Error>((tokens, balances))
        }
        .await;
        match result {
            Ok((tokens, balances)) => {
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
    pub async fn devices(&self) -> Result<(Vec<GpuInfo>, Option<String>)> {
        match crate::process::inspect(&self.snapshot().settings.engine_path).await {
            Ok((_, devices, warning)) => Ok((devices, warning)),
            Err(error) => Ok((vec![], Some(public_error(&error)))),
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
        let (_, devices, _) = crate::process::inspect(&settings.engine_path).await?;
        let gpu = if settings.device == "cpu" {
            None
        } else {
            pengwallet_protocol::preferred_gpu(
                &devices,
                if settings.device == "gpu:auto" {
                    None
                } else {
                    Some(&settings.device)
                },
            )
            .map(Some)
            .context(
                "No supported GPU selected. Choose an available device; CPU fallback is disabled.",
            )?
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
        let cancel = Arc::new(Cancellation::new());
        let child = Arc::clone(&cancel);
        let engine = Arc::clone(self);
        let handle = tokio::spawn(async move {
            engine.run(api, session, settings, gpu, child).await;
        });
        *self.task.lock().await = Some(Task { cancel, handle });
        self.emit(
            "engine_starting",
            Some("Starting the PengWallet CLI mining process."),
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
        let request = Request::Mine {
            version: VERSION,
            server: settings.server.clone(),
            wallet_id: session.wallet_id.clone(),
            cookie: session.cookie.clone(),
            token: settings.token.clone(),
            network: settings.network.clone(),
            gpu: gpu.is_some(),
            gpu_device: gpu,
            threads: settings.cpu_threads,
            intensity: settings.intensity,
        };
        let mut accepted = std::collections::HashSet::new();
        let result = crate::process::mine(
            &settings.engine_path,
            request,
            &cancel.notify,
            cancel.flag.load(Ordering::Relaxed),
            |event| self.cli_event(&api, &session, &settings, event, &mut accepted),
        )
        .await;
        {
            let mut state = self.state.lock().unwrap();
            state.snapshot.running = false;
            state.snapshot.current_hashrate = 0.0;
            state.snapshot.job = None;
        }
        if let Err(error) = result {
            let message = public_error(&error);
            let reported = {
                let mut state = self.state.lock().unwrap();
                let reported = state.snapshot.error.is_some();
                if !reported {
                    state.snapshot.error = Some(message.clone());
                }
                reported
            };
            if !reported {
                self.emit("mining_error", Some(&message));
            }
        }
        self.emit(
            "mining_stopped",
            Some("PengWallet engine stopped. The server balance is authoritative."),
        );
    }
    fn cli_event(
        self: &Arc<Self>,
        api: &Api,
        session: &Session,
        settings: &Settings,
        event: CliEvent,
        accepted: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        match event {
            CliEvent::Ready { cpu_count, .. } => {
                self.state.lock().unwrap().snapshot.cpu_count = cpu_count;
            }
            CliEvent::Started { device_name } => {
                self.state.lock().unwrap().snapshot.device_name = device_name;
                self.emit(
                    "mining_started",
                    Some("PengWallet CLI mining engine started."),
                );
            }
            CliEvent::JobReceived { job } => {
                if job.wallet_id != session.wallet_id {
                    bail!("PengWallet reported a job for a different wallet");
                }
                {
                    let mut state = self.state.lock().unwrap();
                    state.snapshot.difficulty = Some(job.difficulty);
                    state.snapshot.job = Some(job);
                }
                self.emit(
                    "mining_job_received",
                    Some("PengWallet received a fresh mining job."),
                );
            }
            CliEvent::Hashrate {
                hashes,
                elapsed,
                current,
                average,
            } => {
                if ![elapsed, current, average]
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0)
                {
                    bail!("PengWallet reported invalid telemetry");
                }
                {
                    let mut state = self.state.lock().unwrap();
                    let snapshot = &mut state.snapshot;
                    if hashes < snapshot.hashes || elapsed < snapshot.uptime {
                        bail!("PengWallet telemetry moved backwards");
                    }
                    snapshot.hashes = hashes;
                    snapshot.uptime = elapsed;
                    snapshot.current_hashrate = current;
                    snapshot.average_hashrate = average;
                }
                self.emit("hashrate_updated", None);
            }
            CliEvent::SolutionFound { job_id, nonce } => {
                if self.snapshot().job.as_ref().map(|job| &job.job_id) != Some(&job_id)
                    || nonce.parse::<u64>().is_err()
                {
                    bail!("PengWallet reported an invalid solution event");
                }
                self.emit(
                    "solution_found",
                    Some("PengWallet found a proof. Awaiting server verification."),
                );
            }
            CliEvent::RewardAccepted {
                job_id,
                nonce,
                token,
                reward,
                ..
            } => {
                if token != settings.token
                    || self.snapshot().job.as_ref().map(|job| &job.job_id) != Some(&job_id)
                    || !accepted.insert(job_id.clone())
                {
                    bail!("PengWallet reported a duplicate or mismatched reward receipt");
                }
                let history = {
                    let mut state = self.state.lock().unwrap();
                    let snapshot = &mut state.snapshot;
                    snapshot.accepted += 1;
                    snapshot.earnings = snapshot
                        .earnings
                        .checked_add(reward)
                        .context("Earnings overflow")?;
                    snapshot.history.push(Solution {
                        time: now(),
                        server: session.server.clone(),
                        wallet_id: session.wallet_id.clone(),
                        token,
                        job_id,
                        nonce,
                        accepted: true,
                        reward,
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
                    self.emit(
                        "storage_error",
                        Some("Reward accepted, but local history could not be saved."),
                    );
                }
                self.emit(
                    "reward_accepted",
                    Some("PengWallet reported a server-accepted reward."),
                );
                let engine = Arc::clone(self);
                let api = api.clone();
                let session = session.clone();
                tokio::spawn(async move {
                    if let Ok(balances) = api.balances(&session).await {
                        let mut state = engine.state.lock().unwrap();
                        if state.snapshot.wallet_id.as_deref() == Some(&session.wallet_id)
                            && state.snapshot.settings.server == session.server
                        {
                            state.snapshot.balances = balances;
                        }
                    }
                });
            }
            CliEvent::SubmissionRejected { job_id, nonce, .. } => {
                self.reject(
                    session,
                    &settings.token,
                    &job_id,
                    nonce.parse().context("Invalid rejected nonce")?,
                );
            }
            CliEvent::NetworkDisconnected { .. } => {
                self.state.lock().unwrap().snapshot.connected = false;
                self.emit(
                    "network_disconnected",
                    Some("PengWallet lost its connection to PengChain."),
                );
            }
            CliEvent::Error { message } => {
                self.state.lock().unwrap().snapshot.error = Some(message.clone());
                self.emit("mining_error", Some(&message));
            }
            CliEvent::Stopped { hashes, elapsed } => {
                if !elapsed.is_finite() || elapsed < 0.0 {
                    bail!("Invalid engine stop summary");
                }
                let mut state = self.state.lock().unwrap();
                state.snapshot.hashes = hashes;
                state.snapshot.uptime = elapsed;
                state.snapshot.current_hashrate = 0.0;
                state.snapshot.average_hashrate = hashes as f64 / elapsed.max(0.001);
            }
        }
        Ok(())
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
