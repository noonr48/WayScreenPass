//! Host configuration management

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use tracing::{debug, info, warn};

fn config_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    path.push("remote-desktop");
    fs::create_dir_all(&path).ok();
    path.push("hosts.toml");
    path
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    pub name: String,
    pub hostname: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 { 3389 }

impl HostConfig {
    pub fn new(name: String, hostname: String, port: Option<u16>) -> Self {
        Self { name, hostname, port: port.unwrap_or(3389) }
    }

    pub fn address(&self) -> String {
        if self.port == 3389 {
            self.hostname.clone()
        } else {
            format!("{}:{}", self.hostname, self.port)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostsConfig {
    #[serde(default)]
    pub hosts: Vec<HostConfig>,
}

impl HostsConfig {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            debug!("Config not found at {:?}", path);
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        let config: HostsConfig = toml::from_str(&content)?;
        info!("Loaded {} hosts from {:?}", config.hosts.len(), path);
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;
        info!("Saved {} hosts to {:?}", self.hosts.len(), path);
        Ok(())
    }

    pub fn add_host(&mut self, host: HostConfig) {
        if self.hosts.iter().any(|h| h.name == host.name) {
            warn!("Host '{}' already exists", host.name);
            return;
        }
        self.hosts.push(host);
    }

    pub fn remove_host(&mut self, name: &str) -> bool {
        let initial_len = self.hosts.len();
        self.hosts.retain(|h| h.name != name);
        self.hosts.len() != initial_len
    }
}
