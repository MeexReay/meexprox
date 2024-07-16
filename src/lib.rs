use log::{error, info};
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
    pub no_pf_for_ip_connect: bool,
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

pub struct ProxyPlayer {
    pub client_conn: MinecraftConnection<TcpStream>,
    pub server_conn: MinecraftConnection<TcpStream>,
    pub name: Option<String>,
    pub uuid: Option<Uuid>,
    pub protocol_version: u16,
    pub server: Option<ProxyServer>,
}

impl ProxyPlayer {
    pub fn new(
        client_conn: MinecraftConnection<TcpStream>,
        server_conn: MinecraftConnection<TcpStream>,
        name: Option<String>,
        uuid: Option<Uuid>,
        protocol_version: u16,
        server: Option<ProxyServer>,
    ) -> ProxyPlayer {
        ProxyPlayer {
            client_conn,
            server_conn,
            name,
            uuid,
            protocol_version,
            server,
        }
    }
}

pub struct MeexProx {
    pub config: ProxyConfig,
    pub players: Vec<Arc<Mutex<ProxyPlayer>>>,
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

    pub fn get_player(&self, uuid: Uuid) -> Option<Arc<Mutex<ProxyPlayer>>> {
        for player in &self.players {
            if let Some(player_uuid) = player.lock().unwrap().uuid {
                if player_uuid == uuid {
                    return Some(player.clone());
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
            .or(server_config.default_server)
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
            loop {
                server_conn.write_packet(&client_conn.read_packet()?)?;
                client_conn.write_packet(&server_conn.read_packet()?)?;
            }
        } else if next_state == 2 {
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

            let player = Arc::new(Mutex::new(ProxyPlayer::new(
                client_conn.try_clone().unwrap(),
                server_conn.try_clone().unwrap(),
                None,
                None,
                protocol_version,
                Some(server.clone()),
            )));

            this.lock().unwrap().players.push(player.clone());

            thread::spawn({
                let mut client_conn = client_conn.try_clone().unwrap();
                let mut server_conn = server_conn.try_clone().unwrap();

                let player = player.clone();
                let server = server.clone();

                let this = this.clone();

                move || {
                    let res = || -> Result<(), ProtocolError> {
                        let mut joined = false;

                        loop {
                            if let Some(player_server) = player.lock().unwrap().server.as_ref() {
                                if player_server.host != server.host {
                                    break;
                                }
                            } else {
                                break;
                            }

                            let mut packet = match client_conn.read_packet() {
                                Ok(packet) => packet,
                                Err(_) => break,
                            };

                            if packet.id() == 0x00 && !joined {
                                let name = packet.read_string()?;
                                let uuid = packet.read_uuid()?;

                                player.lock().unwrap().name = Some(name.clone());
                                player.lock().unwrap().uuid = Some(uuid.clone());

                                info!(
                                    "{} connected player {} ({})",
                                    addr.to_string(),
                                    &name,
                                    &uuid
                                );

                                joined = true;
                            }

                            server_conn.write_packet(&packet)?;
                        }

                        Ok(())
                    }();

                    if res.is_err() {
                        client_conn.close();
                        server_conn.close();

                        if this.lock().unwrap().remove_player(player.clone()) {
                            match player.lock().unwrap().name.clone() {
                                Some(name) => {
                                    info!("{} disconnected player {}", addr.to_string(), name)
                                }
                                None => {}
                            };
                        }
                    }
                }
            });

            let res = || -> Result<(), ProtocolError> {
                loop {
                    if let Some(player_server) = player.lock().unwrap().server.as_ref() {
                        if player_server.host != server.host {
                            break;
                        }
                    } else {
                        break;
                    }

                    let mut packet = match server_conn.read_packet() {
                        Ok(packet) => packet,
                        Err(_) => break,
                    };

                    if packet.id() == 0x02 {
                        if let PlayerForwarding::PluginResponse = server_config.player_forwarding {
                            server_conn.write_packet(&plugin_response_packet)?;
                        }
                    }

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

                Ok(())
            }();

            if res.is_err() {
                client_conn.close();
                server_conn.close();

                if this.lock().unwrap().remove_player(player.clone()) {
                    match player.lock().unwrap().name.clone() {
                        Some(name) => info!("{} disconnected player {}", addr.to_string(), name),
                        None => {}
                    };
                }
            }
        }

        Ok(())
    }

    pub fn remove_player(&mut self, player: Arc<Mutex<ProxyPlayer>>) -> bool {
        match self.players.iter().position(|x| Arc::ptr_eq(x, &player)) {
            Some(i) => {
                self.players.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn start(self) {
        let listener = TcpListener::bind(&self.config.host).expect("invalid host");

        info!("meexprox started on {}", &self.config.host);

        let mutex_self = Arc::new(Mutex::new(self));

        for client in listener.incoming() {
            if let Ok(client) = client {
                let mutex_self_clone = mutex_self.clone();
                thread::spawn(move || {
                    match Self::accept(mutex_self_clone, client) {
                        Ok(_) => {}
                        Err(e) => {
                            // error!("connection error: {:?}", e);
                        }
                    };
                });
            }
        }
    }
}
