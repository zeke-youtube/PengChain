use anyhow::{Context, Result};
use pengminer::{
    api::{validate_server, Api, Session},
    engine::{Engine, MiningEvent},
    storage::{Files, SessionStore, Settings},
};
use pengwallet::gpu::{preferred_device, GpuInfo};
use std::{
    collections::HashMap,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, String>>);
impl SessionStore for MemoryStore {
    fn load(&self, server: &str) -> Result<Option<Session>> {
        self.0
            .lock()
            .unwrap()
            .get(server)
            .map(|v| serde_json::from_str(v).map_err(Into::into))
            .transpose()
    }
    fn save(&self, session: &Session) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(session.server.clone(), serde_json::to_string(session)?);
        Ok(())
    }
    fn clear(&self, server: &str) -> Result<()> {
        self.0.lock().unwrap().remove(server);
        Ok(())
    }
}
struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn validates_origins_settings_and_gpu_selection() {
    assert!(validate_server("https://peng.example").is_ok());
    for url in [
        "http://peng.example",
        "https://user:secret@example.com",
        "file:///tmp/wallet",
        "https://example.com/path",
        "https://example.com/?token=x",
    ] {
        assert!(validate_server(url).is_err(), "{url}");
    }
    let mut settings = Settings::default();
    assert!(!settings.start_on_launch);
    settings.device = "ppu".into();
    assert!(settings.validate().is_err());
    let integrated = GpuInfo {
        id: "opencl:0".into(),
        index: 0,
        name: "Integrated graphics".into(),
        vendor: "Test".into(),
        device_type: "GPU".into(),
        memory_bytes: 1024,
        integrated: true,
        available: true,
    };
    let dedicated = GpuInfo {
        id: "opencl:1".into(),
        index: 1,
        integrated: false,
        ..integrated.clone()
    };
    let devices = vec![integrated, dedicated];
    assert_eq!(preferred_device(&devices, None).unwrap(), 1);
    assert_eq!(preferred_device(&devices, Some("opencl:0")).unwrap(), 0);
    assert!(preferred_device(&devices, Some("opencl:99")).is_err());
    assert!(preferred_device(&[], None).is_err());
}
#[tokio::test]
async fn real_server_start_stop_wallet_restart_and_network_errors() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let temp = tempfile::tempdir()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let server = Server(
        Command::new("node")
            .arg("dist/index.js")
            .current_dir(root.join("server"))
            .env("PENGCHAIN_DB", temp.path().join("test.sqlite"))
            .env("HOST", "127.0.0.1")
            .env("PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Build the server and put Node 22 on PATH")?,
    );
    let address = format!("http://127.0.0.1:{port}");
    let api = Api::new(&address)?;
    for attempt in 0..100 {
        if api.health().await.is_ok() {
            break;
        }
        if attempt == 99 {
            anyhow::bail!("Server not ready");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let value: serde_json::Value = reqwest::Client::new()
        .post(format!("{address}/blockchain/api/wallet/create"))
        .json(&serde_json::json!({"password":"desktop-test-password"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let wallet = value["walletId"].as_str().unwrap().to_owned();
    let events = Arc::new(Mutex::new(Vec::<MiningEvent>::new()));
    let event_copy = Arc::clone(&events);
    let store = Arc::new(MemoryStore::default());
    let engine = Engine::new(
        store.clone(),
        Files {
            directory: temp.path().join("app"),
        },
        Arc::new(move |event| event_copy.lock().unwrap().push(event)),
    )?;
    let settings = Settings {
        server: address,
        ..Settings::default()
    };
    engine.save_settings(settings).await?;
    assert!(engine
        .login(wallet.clone(), "incorrect-password".into())
        .await
        .is_err());
    engine
        .login(wallet.clone(), "desktop-test-password".into())
        .await?;
    assert!(engine.snapshot().persistent);
    assert_eq!(engine.refresh().await?.tokens[0].symbol, "PENG");
    engine.start().await?;
    assert!(engine.start().await.is_err());
    tokio::time::timeout(Duration::from_secs(20), async {
        while engine.snapshot().accepted < 2 {
            assert!(
                engine.snapshot().error.is_none(),
                "{:?}",
                engine.snapshot().error
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    let stopped = engine.stop().await?;
    assert!(!stopped.running);
    assert!(stopped.hashes > 0);
    assert_eq!(stopped.earnings, 20);
    assert_eq!(stopped.balances[0].amount, 20);
    assert!(stopped.average_hashrate > 0.0);
    let restarted = Engine::new(
        store.clone(),
        Files {
            directory: temp.path().join("app"),
        },
        Arc::new(|_| {}),
    )?;
    let restored = restarted.initialize().await?;
    assert_eq!(restored.wallet_id.as_deref(), Some(wallet.as_str()));
    assert!(!restored.running);
    assert_eq!(restored.history.len(), 2);
    assert_eq!(restored.balances[0].amount, 20);
    let serialized = serde_json::to_string(&engine.snapshot())?;
    assert!(!serialized.contains("desktop-test-password"));
    assert!(!serialized.contains("peng_session="));
    {
        let events = events.lock().unwrap();
        for kind in [
            "mining_started",
            "mining_job_received",
            "solution_found",
            "reward_accepted",
            "hashrate_updated",
            "mining_stopped",
        ] {
            assert!(
                events.iter().any(|e| e.kind == kind),
                "missing event {kind}"
            );
        }
    }
    let session = store.load(&engine.snapshot().settings.server)?.unwrap();
    reqwest::Client::new().post(format!("{}/blockchain/api/token",session.server)).header("Cookie",&session.cookie).json(&serde_json::json!({"name":"Cancellation test","symbol":"HARD","mineable":true,"maxSupply":100,"difficulty":64})).send().await?.error_for_status()?;
    let mut settings = engine.snapshot().settings;
    settings.token = "HARD".into();
    engine.save_settings(settings).await?;
    engine.start().await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let stopped = tokio::time::timeout(Duration::from_secs(3), engine.stop()).await??;
    assert!(!stopped.running);
    assert_eq!(stopped.accepted, 0);
    assert!(stopped.hashes > 0);
    drop(server);
    assert!(engine.refresh().await.is_err());
    assert!(!engine.snapshot().connected);
    engine.logout().await?;
    assert!(store.load(&session.server)?.is_none());
    Ok(())
}

#[tokio::test]
async fn enumeration_handles_missing_driver_without_cpu_fallback() -> Result<()> {
    let (devices, warning) = Engine::devices().await?;
    if warning.is_some() {
        assert!(devices.is_empty());
    }
    for device in &devices {
        assert_eq!(device.device_type, "GPU");
        assert_eq!(device.id, format!("opencl:{}", device.index));
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_credential_manager_round_trip() -> Result<()> {
    use pengminer::storage::OsSessionStore;
    let server = format!("https://credential-test-{}.invalid", std::process::id());
    let store = OsSessionStore;
    let session = Session {
        server: server.clone(),
        wallet_id: "PENG-TEST".into(),
        cookie: "peng_session=test-only-not-a-real-session".into(),
    };
    store.save(&session)?;
    let loaded = store
        .load(&server)?
        .context("Test credential was not restored")?;
    store.clear(&server)?;
    assert_eq!(loaded.wallet_id, session.wallet_id);
    assert_eq!(loaded.cookie, session.cookie);
    assert!(store.load(&server)?.is_none());
    Ok(())
}

#[tokio::test]
async fn rejected_submission_stops_and_emits_an_error() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let server = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let request_count = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let count = stream.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..count]);
                if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let header_end = buffer.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let length = String::from_utf8_lossy(&buffer[..header_end])
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while buffer.len() < header_end + length {
                let count = stream.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..count]);
            }
            let header = String::from_utf8_lossy(&buffer);
            let path = header.lines().next().unwrap_or("");
            let (status, body, cookie) = if path.contains("/wallet/login") {
                (
                    "200 OK",
                    serde_json::json!({"walletId":"PENG-TEST"}),
                    "Set-Cookie: peng_session=test-only; HttpOnly\r\n",
                )
            } else if path.contains("/wallet/balance") {
                ("200 OK", serde_json::json!([]), "")
            } else if path.contains("/mining/job") {
                (
                    "200 OK",
                    serde_json::json!({"jobId":"job","tokenId":"PENG","walletId":"PENG-TEST","challenge":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","difficulty":1,"expiresAt":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64+60_000}),
                    "",
                )
            } else {
                request_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                (
                    "400 Bad Request",
                    serde_json::json!({"error":"invalid solution"}),
                    "",
                )
            };
            let body = body.to_string();
            let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    let directory = tempfile::tempdir()?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::clone(&events);
    let engine = Engine::new(
        Arc::new(MemoryStore::default()),
        Files {
            directory: directory.path().into(),
        },
        Arc::new(move |event| output.lock().unwrap().push(event)),
    )?;
    engine
        .save_settings(Settings {
            server,
            ..Settings::default()
        })
        .await?;
    engine
        .login("PENG-TEST".into(), "test-password".into())
        .await?;
    engine.start().await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while engine.snapshot().running {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert_eq!(engine.snapshot().accepted, 0);
    assert_eq!(engine.snapshot().rejected, 1);
    assert_eq!(engine.snapshot().earnings, 0);
    assert_eq!(requests.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.kind == "submission_rejected"));
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.kind == "mining_error"));
    engine.stop().await?;
    task.abort();
    Ok(())
}
