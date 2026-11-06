//! Only the PengWallet executable computes proofs and submits rewards. This
//! controller launches it directly (no shell), uses private pipes, and reaps it.
use anyhow::{bail, Context, Result};
use pengwallet_protocol::{Event, Frame, Request, MAX_FRAME, VERSION};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::Notify,
};

pub fn executable(configured: &str) -> Result<PathBuf> {
    let path = if configured.is_empty() {
        std::env::current_exe()?
            .parent()
            .context("Could not locate PengMiner")?
            .join(if cfg!(windows) {
                "pengwallet.exe"
            } else {
                "pengwallet"
            })
    } else {
        let path = Path::new(configured);
        if !path.is_absolute() {
            bail!("PengWallet executable path must be absolute");
        }
        path.into()
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !name.eq_ignore_ascii_case(if cfg!(windows) {
        "pengwallet.exe"
    } else {
        "pengwallet"
    }) {
        bail!("Select the trusted PengWallet executable (pengwallet.exe on Windows)");
    }
    let path=path.canonicalize().context("PengWallet CLI was not found. Reinstall PengMiner or select your installed PengWallet executable in Settings.")?;
    if !path.is_file() {
        bail!("PengWallet executable is not a file");
    }
    Ok(path)
}
fn spawn(path: &Path) -> Result<Child> {
    let mut command = Command::new(path);
    if let Some(directory) = path.parent() {
        command.current_dir(directory);
    }
    command
        .args(["engine", "--json", "--no-color"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    command
        .spawn()
        .context("Could not launch PengWallet. Check the executable path and permissions.")
}
async fn frame(reader: &mut BufReader<tokio::process::ChildStdout>) -> Result<Option<Frame>> {
    let mut bytes = Vec::new();
    let length = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if length == 0 {
        return Ok(None);
    }
    if length > MAX_FRAME {
        bail!("PengWallet sent an oversized engine message");
    }
    let value:Frame=serde_json::from_slice(&bytes).context("PengWallet does not support the required JSON engine protocol. Install the matching CLI version.")?;
    if value.protocol_version != VERSION {
        bail!("PengWallet engine protocol version does not match PengMiner");
    }
    Ok(Some(value))
}
async fn write(input: &mut tokio::process::ChildStdin, request: &Request) -> Result<()> {
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    input
        .write_all(&bytes)
        .await
        .context("PengWallet engine input closed unexpectedly")?;
    input.flush().await?;
    Ok(())
}
async fn wait(child: &mut Child) -> Result<std::process::ExitStatus> {
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(result) => Ok(result?),
        Err(_) => {
            let _ = child.kill().await;
            bail!("PengWallet did not exit; the engine was terminated and reaped");
        }
    }
}
fn discard_errors(child: &mut Child) -> tokio::task::JoinHandle<()> {
    let mut stderr = child.stderr.take().expect("piped stderr");
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
    })
}
pub async fn inspect(
    configured: &str,
) -> Result<(usize, Vec<pengwallet_protocol::Device>, Option<String>)> {
    let path = executable(configured)?;
    let mut child = spawn(&path)?;
    let errors = discard_errors(&mut child);
    let result = async {
        let mut input = child.stdin.take().context("Missing engine input")?;
        write(&mut input, &Request::Inspect { version: VERSION }).await?;
        drop(input);
        let mut output = BufReader::new(child.stdout.take().context("Missing engine output")?);
        match frame(&mut output)
            .await?
            .context("PengWallet exited without a protocol handshake")?
            .event
        {
            Event::Ready {
                cpu_count,
                devices,
                warning,
            } => Ok((cpu_count, devices, warning)),
            _ => bail!("PengWallet sent an invalid engine handshake"),
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(30), result).await;
    let result = match result {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            Err(anyhow::anyhow!("PengWallet device discovery timed out"))
        }
    };
    if result.is_err() {
        let _ = child.kill().await;
    } else if !wait(&mut child).await?.success() {
        bail!("PengWallet could not enumerate devices");
    }
    errors.abort();
    result
}

pub async fn mine(
    configured: &str,
    request: Request,
    stop: &Notify,
    already_stopped: bool,
    mut event: impl FnMut(Event) -> Result<()>,
) -> Result<()> {
    let path = executable(configured)?;
    let mut child = spawn(&path)?;
    let errors = discard_errors(&mut child);
    let mut input = child.stdin.take().context("Missing engine input")?;
    let mut output = BufReader::new(child.stdout.take().context("Missing engine output")?);
    let result=async {
        write(&mut input,&request).await?;
        let mut stopping=false;let mut deadline=None;let mut ready=false;let mut stopped=false;let mut failure=None;
        if already_stopped{write(&mut input,&Request::Stop).await?;stopping=true;deadline=Some(tokio::time::Instant::now()+Duration::from_secs(10));}
        loop {tokio::select! {
            _=stop.notified(),if !stopping=>{
                write(&mut input,&Request::Stop).await?;
                stopping=true;deadline=Some(tokio::time::Instant::now()+Duration::from_secs(10));
            }
            _=async{if let Some(deadline)=deadline{tokio::time::sleep_until(deadline).await}else{std::future::pending::<()>().await}}=>{
                bail!("PengWallet did not stop within 10 seconds; the process was terminated. Check your wallet balance for any in-flight submission.");
            }
            value=frame(&mut output)=>{
                let Some(value)=value? else{break;};
                if !ready && !matches!(value.event,Event::Ready{..}){bail!("PengWallet did not send a valid protocol handshake");}
                if matches!(value.event,Event::Ready{..}){if ready{bail!("PengWallet sent a duplicate handshake");}ready=true;}
                if let Event::Error{ref message}=value.event{failure=Some(message.clone());}
                if matches!(value.event,Event::Stopped{..}){stopped=true;}
                event(value.event)?;
                if stopped{break;}
            }
        }}
        let status=wait(&mut child).await?;
        if let Some(message)=failure{bail!("{message}");}
        if !status.success()||!stopped{bail!("PengWallet engine exited unexpectedly. Mining has stopped.");}Ok(())
    }.await;
    if result.is_err() {
        let _ = child.kill().await;
    }
    errors.abort();
    result
}
