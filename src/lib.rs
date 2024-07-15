use log::{debug, error, info};
use rust_mc_proto::{
    DataBufferReader, DataBufferWriter, MinecraftConnection, Packet, ProtocolError, Zigzag,
};
use serde_yml::Value;
use std::{
    error::Error,
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct ProxyServer {
    pub name: String,
    pub host: String,
    pub forced_host: Option<String>,
}

impl ProxyServer {
    pub fn new(name: String, host: String, forced_host: Option<String>) -> ProxyServer {
        ProxyServer {
            name,
            host,
            forced_host,
        }
    }
}

#[derive(Debug)]
pub enum ProxyError {
    ConfigParse,
    ServerConnect,
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({:?})", self)
    }
}

impl std::error::Error for ProxyError {}

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
    PluginResponse,
    Disabled,
}

#[derive(Clone)]
pub struct ProxyConfig {
    pub host: String,
    pub servers: Vec<ProxyServer>,
    pub default_server: Option<ProxyServer>,
    pub talk_host: Option<String>,
    pub talk_secret: Option<String>,
    pub player_forwarding: PlayerForwarding,
}

impl ProxyConfig {
    pub fn new(
        host: String,
        servers: Vec<ProxyServer>,
        default_server: Option<ProxyServer>,
        talk_host: Option<String>,
        talk_secret: Option<String>,
        player_forwarding: PlayerForwarding,
    ) -> ProxyConfig {
        ProxyConfig {
            host,
            servers,
            default_server,
            talk_host,
            talk_secret,
            player_forwarding,
        }
    }

    pub fn load(path: &str) -> Result<ProxyConfig, Box<dyn std::error::Error>> {
        let data = serde_yml::from_str::<Value>(&fs::read_to_string(path)?)?;
        let data = data.as_mapping().ok_or(ProxyError::ConfigParse)?;

        let host = extract_string!(data, "host").ok_or(ProxyError::ConfigParse)?;
        let talk_host = extract_string!(data, "talk_host");
        let talk_secret = extract_string!(data, "talk_secret");
        let player_forwarding = match extract_string!(data, "player_forwarding") {
            Some(pf) => match pf.as_str() {
                "plugin response" => PlayerForwarding::PluginResponse,
                "disabled" => PlayerForwarding::Disabled,
                _ => PlayerForwarding::Handshake,
            },
            _ => PlayerForwarding::Handshake,
        };

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

    pub fn get_host(&self) -> &str {
        &self.host
    }

    pub fn get_talk_enabled(&self) -> bool {
        self.talk_host.is_some() && self.talk_secret.is_some()
    }

    pub fn get_talk_host(&self) -> Option<&String> {
        self.talk_host.as_ref()
    }

    pub fn get_default_server(&self) -> Option<ProxyServer> {
        self.default_server.clone()
    }

    pub fn get_talk_secret(&self) -> Option<&String> {
        self.talk_secret.as_ref()
    }
}

pub struct ProxyPlayer {
    pub connection: TcpStream,
    pub connection_server: TcpStream,
    pub name: Option<String>,
    pub uuid: Option<Uuid>,
    pub server: Option<ProxyServer>,
}

impl ProxyPlayer {
    pub fn new(
        connection: TcpStream,
        connection_server: TcpStream,
        name: Option<String>,
        uuid: Option<Uuid>,
        server: Option<ProxyServer>,
    ) -> ProxyPlayer {
        ProxyPlayer {
            connection,
            connection_server,
            name,
            uuid,
            server,
        }
    }
}

pub struct MeexProx {
    pub config: ProxyConfig,
    pub players: Vec<ProxyPlayer>,
    pub listener: Option<TcpListener>,
}

impl MeexProx {
    pub fn new(config: ProxyConfig) -> MeexProx {
        MeexProx {
            config,
            players: Vec::new(),
            listener: None,
        }
    }

    pub fn get_player(&self, uuid: Uuid) -> Option<&ProxyPlayer> {
        for player in &self.players {
            if let Some(player_uuid) = player.uuid {
                if player_uuid == uuid {
                    return Some(player);
                }
            }
        }
        None
    }

    pub fn accept(this: Arc<Mutex<Self>>, stream: TcpStream) -> Result<(), Box<dyn Error>> {
        let Ok(addr) = stream.peer_addr() else {
            return Ok(());
        };

        let server_config = this.lock().unwrap().config.clone();

        let mut client_conn = MinecraftConnection::new(stream);

        // TODO: remove this anti-ipv6 mrakobesie!!
        let SocketAddr::V4(addrv4) = addr else {
            return Ok(());
        };
        debug!(
            "accepted stream {}.{}.{}.{}:{}",
            addrv4.ip().octets()[0],
            addrv4.ip().octets()[1],
            addrv4.ip().octets()[2],
            addrv4.ip().octets()[3],
            addrv4.port()
        );
        // TODO: remove this anti-ipv6 mrakobesie!!

        let mut handshake = client_conn.read_packet()?;

        if handshake.id() != 0x00 {
            return Ok(());
        }

        let protocol_version = handshake.read_u16_varint()?;
        let server_address = handshake.read_string()?;
        let server_port = handshake.read_unsigned_short()?;
        let next_state = handshake.read_u8_varint()?;

        let server = server_config
            .get_server_by_forced_host(&server_address)
            .or(server_config.get_default_server())
            .ok_or(ProxyError::ConfigParse)?;

        let mut server_conn = MinecraftConnection::connect(&server.host)?;

        let handshake = Packet::build(0x00, |handshake| {
            handshake.write_u16_varint(protocol_version)?;
            handshake.write_string(&server_address)?;
            handshake.write_unsigned_short(server_port)?;
            handshake.write_u8_varint(next_state)?;

            if let PlayerForwarding::Handshake = server_config.player_forwarding {
                if let SocketAddr::V4(addr) = addr {
                    handshake.write_boolean(false)?; // is ipv6
                    handshake.write_unsigned_short(addr.port())?; // port
                    handshake.write_bytes(&addr.ip().octets())?; // octets
                } else if let SocketAddr::V6(addr) = addr {
                    handshake.write_boolean(true)?;
                    handshake.write_unsigned_short(addr.port())?;
                    handshake.write_bytes(&addr.ip().octets())?;
                }
            }

            Ok(())
        })?;

        server_conn.write_packet(&handshake)?;

        if next_state == 1 {
            debug!("state: motd");

            loop {
                server_conn.write_packet(&client_conn.read_packet()?)?;
                client_conn.write_packet(&server_conn.read_packet()?)?;
            }
        } else if next_state == 2 {
            debug!("state: login");

            let plugin_response_packet = Packet::build(0x02, |packet| {
                packet.write_i8_varint(-99)?;
                packet.write_boolean(true)?;

                if let SocketAddr::V4(addr) = addr {
                    packet.write_boolean(false)?; // is ipv6
                    packet.write_unsigned_short(addr.port())?; // port
                    packet.write_bytes(&addr.ip().octets())?; // octets
                } else if let SocketAddr::V6(addr) = addr {
                    packet.write_boolean(true)?;
                    packet.write_unsigned_short(addr.port())?;
                    packet.write_bytes(&addr.ip().octets())?;
                }

                Ok(())
            })?;

            thread::spawn({
                let mut client_conn = client_conn.try_clone().unwrap();
                let mut server_conn = server_conn.try_clone().unwrap();

                move || {
                    move || -> Result<(), ProtocolError> {
                        let mut joined = false;

                        loop {
                            let mut packet = match client_conn.read_packet() {
                                Ok(packet) => packet,
                                Err(_) => break,
                            };

                            if packet.id() == 0x00 && !joined {
                                let name = packet.read_string()?;
                                let uuid = packet.read_uuid()?;

                                this.lock().unwrap().players.push(ProxyPlayer::new(
                                    client_conn.get_ref().try_clone().unwrap(),
                                    server_conn.get_ref().try_clone().unwrap(),
                                    Some(name),
                                    Some(uuid),
                                    Some(server.clone()),
                                ));

                                joined = true;
                            }

                            // debug!("[C->S] sending packet {:#04X?} (size: {})", packet.id(), packet.len());
                            server_conn.write_packet(&packet)?;
                        }
                        error!("serverbound error");

                        Ok(())
                    }()
                    .or_else(|e| {
                        error!("serverbound error: {:?}", e);
                        Ok::<(), ()>(())
                    })
                    .unwrap();
                }
            });

            move || -> Result<(), ProtocolError> {
                loop {
                    let mut packet = match server_conn.read_packet() {
                        Ok(packet) => packet,
                        Err(_) => break,
                    };

                    if packet.id() == 0x02 {
                        if let PlayerForwarding::PluginResponse = server_config.player_forwarding {
                            debug!(
                                "[C->S] sending packet {:#04X?} (size: {})",
                                plugin_response_packet.id(),
                                plugin_response_packet.len()
                            );
                            server_conn.write_packet(&plugin_response_packet)?;
                        }
                    }

                    // debug!("[C<-S] sending packet {:#04X?} (size: {})", packet.id(), packet.len());

                    client_conn.write_packet(&packet)?;

                    if packet.id() == 0x03 {
                        let threshold = packet.read_isize_varint()?;

                        if threshold >= 0 {
                            let threshold = threshold.zigzag();

                            server_conn.set_compression(Some(threshold));
                            client_conn.set_compression(Some(threshold));
                        } else {
                            server_conn.set_compression(None);
                            client_conn.set_compression(None);
                        }
                    }
                }
                error!("clientbound error");

                Ok(())
            }()
            .or_else(|e| {
                error!("clientbound error: {:?}", e);
                Ok::<(), ()>(())
            })
            .unwrap();
        }

        Ok(())
    }

    pub fn start(self) {
        let listener = TcpListener::bind(self.config.get_host()).expect("invalid host");

        info!("meexprox started on {}", self.config.get_host());

        let mutex_self = Arc::new(Mutex::new(self));

        for client in listener.incoming() {
            if let Ok(client) = client {
                let mutex_self_clone = mutex_self.clone();
                thread::spawn(move || {
                    Self::accept(mutex_self_clone, client).expect("accept error");
                });
            }
        }
    }
}
