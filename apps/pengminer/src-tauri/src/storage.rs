use crate::api::{validate_server, Session};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

pub trait SessionStore: Send + Sync {
    fn load(&self, server: &str) -> Result<Option<Session>>;
    fn save(&self, session: &Session) -> Result<()>;
    fn clear(&self, server: &str) -> Result<()>;
}
pub struct OsSessionStore;
impl SessionStore for OsSessionStore {
    fn load(&self, server: &str) -> Result<Option<Session>> {
        let entry = keyring::Entry::new("studio.pika.pengminer", server)?;
        match entry.get_password() {
            Ok(value) => {
                let session: Session = serde_json::from_str(&value)?;
                if session.server != server {
                    bail!("Stored session belongs to a different server");
                }
                Ok(Some(session))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn save(&self, session: &Session) -> Result<()> {
        keyring::Entry::new("studio.pika.pengminer", &session.server)?
            .set_password(&serde_json::to_string(session)?)?;
        Ok(())
    }
    fn clear(&self, server: &str) -> Result<()> {
        match keyring::Entry::new("studio.pika.pengminer", server)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub engine_path: String,
    pub server: String,
    pub token: String,
    pub network: String,
    pub device: String,
    pub cpu_threads: usize,
    pub intensity: u8,
    pub start_on_launch: bool,
    pub minimize_to_tray: bool,
    pub theme: String,
    pub notifications: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            engine_path: String::new(),
            server: "https://pengchain.pikastudio.org".into(),
            token: "PENG".into(),
            network: "pengchain-v1".into(),
            device: "cpu".into(),
            cpu_threads: 1,
            intensity: 100,
            start_on_launch: false,
            minimize_to_tray: false,
            theme: "dark".into(),
            notifications: false,
        }
    }
}
impl Settings {
    pub fn validate(&mut self) -> Result<()> {
        self.server = validate_server(&self.server)?;
        self.engine_path = self.engine_path.trim().into();
        if self.engine_path.len() > 4096
            || self.engine_path.chars().any(char::is_control)
            || (!self.engine_path.is_empty()
                && !std::path::Path::new(&self.engine_path).is_absolute())
        {
            bail!("PengWallet engine override must be an absolute executable path");
        }
        let max = std::thread::available_parallelism()?.get();
        if self.cpu_threads == 0 || self.cpu_threads > max {
            bail!("CPU thread count must be between 1 and {max}");
        }
        if ![25, 50, 100].contains(&self.intensity) {
            bail!("Intensity must be 25, 50, or 100 percent");
        }
        if self.token.is_empty()
            || self.token.len() > 16
            || !self.token.bytes().all(|c| c.is_ascii_alphanumeric())
            || !self.token.as_bytes()[0].is_ascii_alphabetic()
        {
            bail!("Enter a valid token symbol");
        }
        if self.network.is_empty() || self.network.len() > 128 {
            bail!("Network ID must contain 1–128 characters");
        }
        if self.device != "cpu"
            && self.device != "gpu:auto"
            && !self
                .device
                .strip_prefix("opencl:")
                .is_some_and(|v| v.parse::<usize>().is_ok())
        {
            bail!("Select a supported CPU or GPU; PPU mining is not implemented");
        }
        if !["dark", "light", "system"].contains(&self.theme.as_str()) {
            bail!("Unsupported theme");
        }
        self.token = self.token.to_uppercase();
        Ok(())
    }
}

pub struct Files {
    pub directory: PathBuf,
}
impl Files {
    pub fn settings(&self) -> Result<Settings> {
        let path = self.directory.join("settings.json");
        let mut settings = if path.exists() {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            Settings::default()
        };
        // An older machine may have more CPU threads than the current one.
        settings.cpu_threads = settings
            .cpu_threads
            .min(std::thread::available_parallelism()?.get())
            .max(1);
        settings.validate()?;
        Ok(settings)
    }
    pub fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        fs::create_dir_all(&self.directory)?;
        let file = self.directory.join(name);
        let temporary = self.directory.join(format!("{name}.tmp"));
        fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
        // Replace is atomic on Unix; Windows requires removing an existing target.
        if cfg!(windows) && file.exists() {
            fs::remove_file(&file).context("Could not replace settings")?;
        }
        fs::rename(temporary, file)?;
        Ok(())
    }
}
