use serde_yml::Value;
use std::fs;
use std::path::Path;

use super::error::ProxyError;

#[derive(Clone, Debug)]
pub struct ServerInfo {
    pub name: String,
    pub host: String,
    pub forced_host: Option<String>,
}

impl ServerInfo {
    pub fn new(name: String, host: String, forced_host: Option<String>) -> ServerInfo {
        ServerInfo {
            name,
            host,
            forced_host,
        }
    }

    pub fn from_host(host: String) -> ServerInfo {
        ServerInfo {
            name: host.clone(),
            host,
            forced_host: None,
        }
    }
}

#[derive(Clone)]
pub enum PlayerForwarding {
    Handshake,
    Disabled,
}

#[derive(Clone)]
pub struct ProxyConfig {
    pub host: String,
    pub servers: Vec<ServerInfo>,
    pub default_server: Option<ServerInfo>,
    pub talk_host: Option<String>,
    pub talk_secret: Option<String>,
    pub player_forwarding: PlayerForwarding,
    pub no_pf_for_ip_connect: bool,
}

impl ProxyConfig {
    pub fn new(
        host: String,
        servers: Vec<ServerInfo>,
        default_server: Option<ServerInfo>,
        talk_host: Option<String>,
        talk_secret: Option<String>,
        player_forwarding: PlayerForwarding,
        no_pf_for_ip_connect: bool,
    ) -> ProxyConfig {
        ProxyConfig {
            host,
            servers,
            default_server,
            talk_host,
            talk_secret,
            player_forwarding,
            no_pf_for_ip_connect,
        }
    }

    pub fn load_yml(data: String) -> Result<ProxyConfig, Box<dyn std::error::Error>> {
        let data = serde_yml::from_str::<Value>(&data)?;
        let data = data.as_mapping().ok_or(ProxyError::ConfigParse)?;

        let host = data.get("host").map(|o| o.as_str()).flatten().ok_or(ProxyError::ConfigParse)?.to_string();
        let talk_host = data.get("talk_host").map(|o| o.as_str()).flatten().map(|o| o.to_string());
        let talk_secret = data.get("talk_secret").map(|o| o.as_str()).flatten().map(|o| o.to_string());
        let player_forwarding = match data.get("player_forwarding").map(|o| o.as_str()).flatten() {
            Some(pf) => match pf {
                "disabled" => PlayerForwarding::Disabled,
                _ => PlayerForwarding::Handshake,
            },
            _ => PlayerForwarding::Handshake,
        };
        let no_pf_for_ip_connect = data
            .get(Value::String("no_pf_for_ip_connect".to_string()))
            .or(Some(&Value::Bool(true)))
            .ok_or(ProxyError::ConfigParse)?
            .as_bool()
            .ok_or(ProxyError::ConfigParse)?;

        let mut servers = Vec::new();
        if let Some(servers_map) = data
            .get(&Value::String("servers".to_string()))
            .and_then(Value::as_mapping)
        {
            for (name, addr) in servers_map {
                if let (Value::String(name), Value::String(addr)) = (name, addr) {
                    servers.push(ServerInfo::new(name.clone(), addr.clone(), None));
                }
            }
        }

        if let Some(forced_hosts_map) = data
            .get(&Value::String("forced_hosts".to_string()))
            .and_then(Value::as_mapping)
        {
            for (name, host) in forced_hosts_map {
                if let (Value::String(name), Value::String(host)) = (name, host) {
                    if let Some(server) = servers.iter_mut().find(|s| s.name == *name) {
                        server.forced_host = Some(host.clone());
                    }
                }
            }
        }

        let default_server = data.get("default_server")
            .map(|o| o.as_str()).flatten()
            .and_then(|ds| servers.iter().find(|s| s.name == ds).cloned());

        Ok(ProxyConfig::new(
            host,
            servers,
            default_server,
            talk_host,
            talk_secret,
            player_forwarding,
            no_pf_for_ip_connect,
        ))
    }

    pub fn load(path: impl AsRef<Path>) -> Result<ProxyConfig, Box<dyn std::error::Error>> {
        Self::load_yml(fs::read_to_string(path)?)
    }

    pub fn get_server_by_name(&self, name: &str) -> Option<ServerInfo> {
        for server in &self.servers {
            if &server.name == name {
                return Some(server.clone());
            }
        }
        None
    }

    pub fn get_server_by_forced_host(&self, forced_host: &str) -> Option<ServerInfo> {
        for server in &self.servers {
            if let Some(server_forced_host) = &server.forced_host {
                if server_forced_host == forced_host {
                    return Some(server.clone());
                }
            }
        }
        None
    }
}
