use serde_yml::{Mapping, Value};
use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct ServerInfo {
    pub name: String,
    pub host: String,
    pub domains: Vec<String>,
    pub player_forwarding: PlayerForwarding,
}

impl ServerInfo {
    pub fn new(
        name: String, 
        host: String, 
        domains: Vec<String>, 
        player_forwarding: PlayerForwarding
    ) -> ServerInfo {
        ServerInfo {
            name,
            host,
            domains,
            player_forwarding
        }
    }

    pub fn from_host(host: String, player_forwarding: PlayerForwarding) -> ServerInfo {
        ServerInfo {
            name: String::new(),
            host,
            domains: Vec::new(),
            player_forwarding
        }
    }
}

#[derive(Clone, Debug)]
pub enum PlayerForwarding {
    Velocity(String),
    Bungeecord(Option<String>),
    Meexprox(String),
    None
}

impl PlayerForwarding {
    pub fn from_data(data: Mapping) -> Option<PlayerForwarding> {
        if data.len() == 0 { return None }
        Some(if data.get("enabled")?.as_bool()? {
            match data.get("type")?.as_str()? {
                "velocity" => {
                    PlayerForwarding::Velocity(
                        data.get("secret")?
                            .as_str()?
                            .to_string()
                    )
                }, "bungeecord" => {
                    PlayerForwarding::Bungeecord(
                        data.get("secret")
                            .map(|o| o.as_str())
                            .flatten()
                            .map(|o| o.to_string())
                    )
                }, "meexprox" => {
                    PlayerForwarding::Meexprox(
                        data.get("secret")?
                            .as_str()?
                            .to_string()
                    )
                }, _ => {
                    return None;
                }
            }
        } else {
            PlayerForwarding::None
        })
    }
}

#[derive(Clone)]
pub struct Messaging {
    pub host: String,
    pub secret: String
}

#[derive(Clone)]
pub struct ProxyConfig {
    pub host: String,
    pub servers: Vec<ServerInfo>,
    pub messaging: Option<Messaging>,
    pub default_forwarding: PlayerForwarding,
    pub incoming_forwarding: PlayerForwarding
}

impl ProxyConfig {
    pub fn new(
        host: String,
        servers: Vec<ServerInfo>,
        messaging: Option<Messaging>,
        default_forwarding: PlayerForwarding,
        incoming_forwarding: PlayerForwarding
    ) -> ProxyConfig {
        ProxyConfig {
            host,
            servers,
            messaging,
            default_forwarding,
            incoming_forwarding
        }
    }

    pub fn load_yml(data: String) -> Option<ProxyConfig> {
        let data = serde_yml::from_str::<Value>(&data).ok()?;
        let data = data.as_mapping()?;

        let host = data.get("host")?.as_str()?.to_string();
        
        let messaging = if let Some(map) = data.get("messaging") {
            let map = map.as_mapping()?;

            if map.get("enabled")?.as_bool()? { 
                Some(Messaging { 
                    host: map.get("host")?.as_str()?.to_string(),
                    secret: map.get("secret")?.as_str()?.to_string(),
                })
            } else {
                None
            }
        } else {
            None
        };

        let servers: Vec<ServerInfo> = data.get("servers")?.as_mapping()?
            .iter()
            .filter_map(|o| -> Option<ServerInfo> {
                let map = o.1.as_mapping()?;
                Some(ServerInfo::new(
                    o.0.as_str()?.to_string(), 
                    map.get("host")?.as_str()?.to_string(), 
                    map.get("domains")?.as_sequence()?
                        .iter()
                        .filter_map(|o| o.as_str())
                        .map(|o| o.to_string())
                        .collect(), 
                    PlayerForwarding::from_data(
                        map.get("forwarding")?.as_mapping()?.clone()
                    )?
                ))
            })
            .collect();

        let default_forwarding = PlayerForwarding::from_data(
            data.get("default_forwarding")?.as_mapping()?.clone()
        )?;

        let incoming_forwarding = PlayerForwarding::from_data(
            data.get("incoming_forwarding")?.as_mapping()?.clone()
        )?;

        Some(ProxyConfig::new(
            host,
            servers,
            messaging,
            default_forwarding,
            incoming_forwarding
        ))
    }

    pub fn load(path: impl AsRef<Path>) -> Option<ProxyConfig> {
        Self::load_yml(fs::read_to_string(path).ok()?)
    }

    pub fn get_server_by_name(&self, name: &str) -> Option<ServerInfo> {
        for server in &self.servers {
            if &server.name == name {
                return Some(server.clone());
            }
        }
        None
    }

    pub fn get_server_by_domain(&self, domain: &str) -> Option<ServerInfo> {
        for server in &self.servers {
            if server.domains.contains(&domain.to_string()) {
                return Some(server.clone());
            }
        }

        for server in &self.servers {
            if server.domains.contains(&"_".to_string()) {
                return Some(server.clone()); 
            }
        }

        None
    }
}
