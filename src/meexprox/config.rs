use super::ProxyError;
use serde_yml::Value;
use std::fs;

#[derive(Clone, Debug)]
pub struct ProxyServer {
    name: String,
    host: String,
    forced_host: Option<String>,
}

impl ProxyServer {
    pub fn new(name: String, host: String, forced_host: Option<String>) -> ProxyServer {
        ProxyServer {
            name,
            host,
            forced_host,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn forced_host(&self) -> Option<&String> {
        self.forced_host.as_ref()
    }
}

macro_rules! extract_string {
    ($data:expr, $key:expr) => {
        match $data.get(&Value::String($key.to_string())) {
            Some(Value::String(val)) => Some(val.clone()),
            _ => None,
        }
    };
}

#[derive(Clone)]
pub enum PlayerForwarding {
    Handshake,
    Disabled,
}

#[derive(Clone)]
pub struct ProxyConfig {
    host: String,
    servers: Vec<ProxyServer>,
    default_server: Option<ProxyServer>,
    talk_host: Option<String>,
    talk_secret: Option<String>,
    player_forwarding: PlayerForwarding,
    no_pf_for_ip_connect: bool,
}

impl ProxyConfig {
    pub fn new(
        host: String,
        servers: Vec<ProxyServer>,
        default_server: Option<ProxyServer>,
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

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn servers(&self) -> &Vec<ProxyServer> {
        &self.servers
    }

    pub fn talk_host(&self) -> Option<&String> {
        self.talk_host.as_ref()
    }

    pub fn talk_secret(&self) -> Option<&String> {
        self.talk_secret.as_ref()
    }
    pub fn default_server(&self) -> Option<&ProxyServer> {
        self.default_server.as_ref()
    }

    pub fn player_forwarding(&self) -> &PlayerForwarding {
        &self.player_forwarding
    }

    pub fn no_pf_for_ip_connect(&self) -> bool {
        self.no_pf_for_ip_connect
    }

    pub fn load(path: &str) -> Result<ProxyConfig, Box<dyn std::error::Error>> {
        let data = serde_yml::from_str::<Value>(&fs::read_to_string(path)?)?;
        let data = data.as_mapping().ok_or(ProxyError::ConfigParse)?;

        let host = extract_string!(data, "host").ok_or(ProxyError::ConfigParse)?;
        let talk_host = extract_string!(data, "talk_host");
        let talk_secret = extract_string!(data, "talk_secret");
        let player_forwarding = match extract_string!(data, "player_forwarding") {
            Some(pf) => match pf.as_str() {
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
                    servers.push(ProxyServer::new(name.clone(), addr.clone(), None));
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

        let default_server = extract_string!(data, "default_server")
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

    pub fn get_server_by_name(&self, name: &str) -> Option<ProxyServer> {
        for server in &self.servers {
            if &server.name == name {
                return Some(server.clone());
            }
        }
        None
    }

    pub fn get_server_by_forced_host(&self, forced_host: &str) -> Option<ProxyServer> {
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
