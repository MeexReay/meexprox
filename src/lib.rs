use derivative::Derivative;
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
}

#[derive(Debug)]
pub enum ProxyError {
    ConfigParse,
    ServerConnect,
    EventChanged,
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

#[derive(Derivative)]
#[derivative(Debug)]
pub struct ProxyPlayer {
    #[derivative(Debug = "ignore")]
    client_conn: MinecraftConnection<TcpStream>,
    #[derivative(Debug = "ignore")]
    server_conn: MinecraftConnection<TcpStream>,
    name: Option<String>,
    uuid: Option<Uuid>,
    protocol_version: u16,
    server: Option<ProxyServer>,
    shared_secret: Option<Vec<u8>>,
    verify_token: Option<Vec<u8>>,
}

impl ProxyPlayer {
    pub fn new(
        client_conn: MinecraftConnection<TcpStream>,
        server_conn: MinecraftConnection<TcpStream>,
        name: Option<String>,
        uuid: Option<Uuid>,
        protocol_version: u16,
        server: Option<ProxyServer>,
        shared_secret: Option<Vec<u8>>,
        verify_token: Option<Vec<u8>>,
    ) -> ProxyPlayer {
        ProxyPlayer {
            client_conn,
            server_conn,
            name,
            uuid,
            protocol_version,
            server,
            shared_secret,
            verify_token,
        }
    }

    pub fn connect_to_ip(
        player: PlayerMutex,
        this: MeexProxMutex,
        ip: &str,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        let ProxyEvent::PlayerConnectingIPEvent { player: _, ip } = this
            .lock()
            .unwrap()
            .trigger_event(ProxyEvent::PlayerConnectingIPEvent {
                player: player.clone(),
                ip: ip.to_string(),
            })
        else {
            return Ok(());
        };

        Self::connect_to_stream(
            player,
            this,
            TcpStream::connect(ip).or(Err(ProxyError::ServerConnect))?,
            None,
            server_address,
            server_port,
        )
    }

    pub fn connect_to_server(
        player: PlayerMutex,
        this: MeexProxMutex,
        server: ProxyServer,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        let ProxyEvent::PlayerConnectingServerEvent { player: _, server } = this
            .lock()
            .unwrap()
            .trigger_event(ProxyEvent::PlayerConnectingServerEvent {
                player: player.clone(),
                server,
            })
        else {
            return Ok(());
        };

        Self::connect_to_stream(
            player,
            this,
            TcpStream::connect(&server.host).or(Err(ProxyError::ServerConnect))?,
            Some(server),
            server_address,
            server_port,
        )
    }

    pub fn connect_to_stream(
        player: PlayerMutex,
        this: MeexProxMutex,
        stream: TcpStream,
        server: Option<ProxyServer>,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        let addr = stream.peer_addr().unwrap();

        let server_config = this.lock().unwrap().config.clone();

        player.lock().unwrap().server_conn = MinecraftConnection::new(stream);
        player.lock().unwrap().server = server.clone();

        let protocol_version = player.lock().unwrap().protocol_version;

        {
            let server_config = server_config.clone();

            player
                .lock()
                .unwrap()
                .server_conn
                .write_packet(&Packet::build(0x00, move |handshake| {
                    handshake.write_u16_varint(protocol_version)?;
                    handshake.write_string(&server_address)?;
                    handshake.write_unsigned_short(server_port)?;
                    handshake.write_u8_varint(2)?;

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
                })?)?;
        }

        {
            let locked = player.lock().unwrap();

            if let Some(player_name) = locked.name.as_ref() {
                if let Some(player_uuid) = locked.uuid.as_ref() {
                    let login_packet = Packet::build(0x00, move |login| {
                        login.write_string(&player_name)?;
                        login.write_uuid(&player_uuid)?;
                        Ok(())
                    })?;

                    let ProxyEvent::SendServerPacketEvent {
                        packet: login_packet,
                        player: _,
                    } = this
                        .lock()
                        .unwrap()
                        .trigger_event(ProxyEvent::SendServerPacketEvent {
                            packet: login_packet,
                            player: player.clone(),
                        })
                    else {
                        return Ok(());
                    };

                    player
                        .lock()
                        .unwrap()
                        .server_conn
                        .write_packet(&login_packet)?;
                }
            }
        }

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

        let mut client_conn = player.lock().unwrap().client_conn.try_clone().unwrap();
        let mut server_conn = player.lock().unwrap().server_conn.try_clone().unwrap();

        thread::spawn({
            let mut client_conn = client_conn.try_clone().unwrap();
            let mut server_conn = server_conn.try_clone().unwrap();

            let player = player.clone();
            let this = this.clone();
            let server = server.clone();

            move || {
                let res = || -> Result<(), ProtocolError> {
                    loop {
                        if server.is_none() && player.lock().unwrap().server.is_some() {
                            break;
                        } else if let Some(server) = server.clone() {
                            if let Some(player_server) = player.lock().unwrap().server.as_ref() {
                                if player_server.host != server.host {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }

                        let packet = match client_conn.read_packet() {
                            Ok(packet) => packet,
                            Err(_) => break,
                        };

                        let ProxyEvent::RecvClientPacketEvent { packet, player: _ } = this
                            .lock()
                            .unwrap()
                            .trigger_event(ProxyEvent::RecvClientPacketEvent {
                                packet,
                                player: player.clone(),
                            })
                        else {
                            return Ok(());
                        };

                        let ProxyEvent::SendServerPacketEvent { packet, player: _ } = this
                            .lock()
                            .unwrap()
                            .trigger_event(ProxyEvent::SendServerPacketEvent {
                                packet,
                                player: player.clone(),
                            })
                        else {
                            return Ok(());
                        };

                        server_conn.write_packet(&packet)?;
                    }

                    Ok(())
                }();

                if res.is_err() {
                    client_conn.close();
                    server_conn.close();

                    if this.lock().unwrap().remove_player(player.clone()) {
                        if let Some(name) = player.lock().unwrap().name.clone() {
                            info!("{} disconnected player {}", addr.to_string(), name);

                            let ProxyEvent::PlayerDisconnectedEvent { player: _ } = this
                                .lock()
                                .unwrap()
                                .trigger_event(ProxyEvent::PlayerDisconnectedEvent {
                                    player: player.clone(),
                                })
                            else {
                                return;
                            };
                        }
                    }
                }
            }
        });

        let res = || -> Result<(), ProtocolError> {
            let mut logged = false;

            loop {
                if server.is_none() && player.lock().unwrap().server.is_some() {
                    break;
                } else if let Some(server) = server.clone() {
                    if let Some(player_server) = player.lock().unwrap().server.as_ref() {
                        if player_server.host != server.host {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                let packet = match server_conn.read_packet() {
                    Ok(packet) => packet,
                    Err(_) => break,
                };

                let ProxyEvent::RecvServerPacketEvent {
                    mut packet,
                    player: _,
                } = this
                    .lock()
                    .unwrap()
                    .trigger_event(ProxyEvent::RecvServerPacketEvent {
                        packet,
                        player: player.clone(),
                    })
                else {
                    return Ok(());
                };

                if packet.id() == 0x02 && !logged {
                    if let PlayerForwarding::PluginResponse = server_config.player_forwarding {
                        let ProxyEvent::SendServerPacketEvent {
                            packet: plugin_response_packet,
                            player: _,
                        } = this
                            .lock()
                            .unwrap()
                            .trigger_event(ProxyEvent::SendServerPacketEvent {
                                packet: plugin_response_packet.clone(),
                                player: player.clone(),
                            })
                        else {
                            return Ok(());
                        };

                        server_conn.write_packet(&plugin_response_packet)?;
                    }
                    logged = true;

                    continue;
                }

                if packet.id() == 0x01 && !logged {
                    let locked = player.lock().unwrap();

                    if let Some(shared_secret) = locked.shared_secret.as_ref() {
                        if let Some(verify_token) = locked.verify_token.as_ref() {
                            let encryption_response = Packet::build(0x00, move |resp| {
                                resp.write_usize_varint(shared_secret.len())?;
                                resp.write_bytes(&shared_secret)?;
                                resp.write_usize_varint(verify_token.len())?;
                                resp.write_bytes(&verify_token)?;
                                Ok(())
                            })?;

                            let ProxyEvent::SendServerPacketEvent {
                                packet: encryption_response,
                                player: _,
                            } = this.lock().unwrap().trigger_event(
                                ProxyEvent::SendServerPacketEvent {
                                    packet: encryption_response,
                                    player: player.clone(),
                                },
                            )
                            else {
                                return Ok(());
                            };

                            server_conn.write_packet(&encryption_response)?;
                        }
                    }

                    continue;
                }

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

                    continue;
                }

                let ProxyEvent::SendClientPacketEvent { packet, player: _ } = this
                    .lock()
                    .unwrap()
                    .trigger_event(ProxyEvent::SendClientPacketEvent {
                        packet,
                        player: player.clone(),
                    })
                else {
                    return Ok(());
                };

                client_conn.write_packet(&packet)?;
            }

            Ok(())
        }();

        if res.is_err() {
            client_conn.close();
            server_conn.close();

            if this.lock().unwrap().remove_player(player.clone()) {
                if let Some(name) = player.lock().unwrap().name.clone() {
                    info!("{} disconnected player {}", addr.to_string(), name);

                    let ProxyEvent::PlayerDisconnectedEvent { player: _ } = this
                        .lock()
                        .unwrap()
                        .trigger_event(ProxyEvent::PlayerDisconnectedEvent {
                            player: player.clone(),
                        })
                    else {
                        return Ok(());
                    };
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum ProxyEvent {
    /// client <- proxy <- server
    ///                 |
    ///                 RecvServerPacketEvent
    RecvServerPacketEvent {
        packet: Packet,
        player: PlayerMutex,
    },

    /// client -> proxy -> server
    ///                 |
    ///                 SendServerPacketEvent
    SendServerPacketEvent {
        packet: Packet,
        player: PlayerMutex,
    },

    /// client <- proxy <- server
    ///        |
    ///        SendClientPacketEvent
    SendClientPacketEvent {
        packet: Packet,
        player: PlayerMutex,
    },

    /// client -> proxy -> server
    ///        |
    ///        RecvClientPacketEvent
    RecvClientPacketEvent {
        packet: Packet,
        player: PlayerMutex,
    },

    PlayerConnectedEvent {
        player: PlayerMutex,
    },

    PlayerConnectingServerEvent {
        player: PlayerMutex,
        server: ProxyServer,
    },

    PlayerConnectingIPEvent {
        player: PlayerMutex,
        ip: String,
    },

    PlayerDisconnectedEvent {
        player: PlayerMutex,
    },

    StatusRequestEvent {
        status: String,
        client_address: SocketAddr,
        server_address: String,
        server_port: u16,
    },
}

pub trait EventListener {
    fn on_event(&mut self, event: &mut ProxyEvent);
}

pub struct MeexProx {
    config: ProxyConfig,
    players: Vec<PlayerMutex>,
    event_listeners: Vec<Box<dyn EventListener + Send + Sync>>,
}

impl MeexProx {
    pub fn new(config: ProxyConfig) -> MeexProx {
        MeexProx {
            config,
            players: Vec::new(),
            event_listeners: Vec::new(),
        }
    }

    pub fn add_event_listener(&mut self, event_listener: Box<dyn EventListener + Send + Sync>) {
        self.event_listeners.push(event_listener);
    }

    pub fn trigger_event(&mut self, mut event: ProxyEvent) -> ProxyEvent {
        for event_listener in &mut self.event_listeners {
            event_listener.on_event(&mut event);
        }
        event
    }

    pub fn get_player(&self, uuid: Uuid) -> Option<PlayerMutex> {
        for player in &self.players {
            if let Some(player_uuid) = player.lock().unwrap().uuid {
                if player_uuid == uuid {
                    return Some(player.clone());
                }
            }
        }
        None
    }

    pub fn remove_player(&mut self, player: PlayerMutex) -> bool {
        match self.players.iter().position(|x| Arc::ptr_eq(x, &player)) {
            Some(i) => {
                self.players.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn accept_client(this: MeexProxMutex, stream: TcpStream) -> Result<(), Box<dyn Error>> {
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
                let client_packet = client_conn.read_packet()?;

                server_conn.write_packet(&client_packet)?;

                let mut server_packet = server_conn.read_packet()?;

                if client_packet.id() == 0x00 {
                    let server_status = server_packet.read_string()?;

                    let ProxyEvent::StatusRequestEvent {
                        status: server_status,
                        client_address: _,
                        server_address: _,
                        server_port: _,
                    } = this
                        .lock()
                        .unwrap()
                        .trigger_event(ProxyEvent::StatusRequestEvent {
                            status: server_status.clone(),
                            client_address: addr.clone(),
                            server_address: server_address.clone(),
                            server_port,
                        })
                    else {
                        return Ok(());
                    };

                    server_packet = Packet::build(0x00, |p| p.write_string(&server_status))?;
                }

                client_conn.write_packet(&server_packet)?;
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
                None,
                None,
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
                        let mut encryption = false;

                        loop {
                            if let Some(player_server) = player.lock().unwrap().server.as_ref() {
                                if player_server.host != server.host {
                                    break;
                                }
                            } else {
                                break;
                            }

                            let packet = match client_conn.read_packet() {
                                Ok(packet) => packet,
                                Err(_) => break,
                            };

                            let ProxyEvent::RecvClientPacketEvent {
                                mut packet,
                                player: _,
                            } = this.lock().unwrap().trigger_event(
                                ProxyEvent::RecvClientPacketEvent {
                                    packet,
                                    player: player.clone(),
                                },
                            )
                            else {
                                return Ok(());
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

                                let ProxyEvent::PlayerConnectedEvent { player: _ } = this
                                    .lock()
                                    .unwrap()
                                    .trigger_event(ProxyEvent::PlayerConnectedEvent {
                                        player: player.clone(),
                                    })
                                else {
                                    return Ok(());
                                };

                                joined = true;
                            }

                            if packet.id() == 0x01 && !encryption {
                                let shared_secret_length = packet.read_usize_varint()?;
                                let shared_secret = packet.read_bytes(shared_secret_length)?;
                                let verify_token_length = packet.read_usize_varint()?;
                                let verify_token = packet.read_bytes(verify_token_length)?;

                                player.lock().unwrap().shared_secret = Some(shared_secret.clone());
                                player.lock().unwrap().verify_token = Some(verify_token.clone());

                                encryption = true;
                            }

                            let ProxyEvent::SendServerPacketEvent { packet, player: _ } = this
                                .lock()
                                .unwrap()
                                .trigger_event(ProxyEvent::SendServerPacketEvent {
                                    packet,
                                    player: player.clone(),
                                })
                            else {
                                return Ok(());
                            };

                            server_conn.write_packet(&packet)?;
                        }

                        Ok(())
                    }();

                    if res.is_err() {
                        client_conn.close();
                        server_conn.close();

                        if this.lock().unwrap().remove_player(player.clone()) {
                            if let Some(name) = player.lock().unwrap().name.clone() {
                                info!("{} disconnected player {}", addr.to_string(), name);

                                let ProxyEvent::PlayerDisconnectedEvent { player: _ } = this
                                    .lock()
                                    .unwrap()
                                    .trigger_event(ProxyEvent::PlayerDisconnectedEvent {
                                        player: player.clone(),
                                    })
                                else {
                                    return;
                                };
                            }
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

                    let packet = match server_conn.read_packet() {
                        Ok(packet) => packet,
                        Err(_) => break,
                    };

                    let ProxyEvent::RecvServerPacketEvent { packet, player: _ } = this
                        .lock()
                        .unwrap()
                        .trigger_event(ProxyEvent::RecvServerPacketEvent {
                            packet,
                            player: player.clone(),
                        })
                    else {
                        return Ok(());
                    };

                    if packet.id() == 0x02 {
                        if let PlayerForwarding::PluginResponse = server_config.player_forwarding {
                            let ProxyEvent::SendServerPacketEvent {
                                packet: plugin_response_packet,
                                player: _,
                            } = this.lock().unwrap().trigger_event(
                                ProxyEvent::SendServerPacketEvent {
                                    packet: plugin_response_packet.clone(),
                                    player: player.clone(),
                                },
                            )
                            else {
                                return Ok(());
                            };

                            server_conn.write_packet(&plugin_response_packet)?;
                        }
                    }

                    let ProxyEvent::SendClientPacketEvent {
                        mut packet,
                        player: _,
                    } = this
                        .lock()
                        .unwrap()
                        .trigger_event(ProxyEvent::SendClientPacketEvent {
                            packet,
                            player: player.clone(),
                        })
                    else {
                        return Ok(());
                    };

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
                    if let Some(name) = player.lock().unwrap().name.clone() {
                        info!("{} disconnected player {}", addr.to_string(), name);

                        let ProxyEvent::PlayerDisconnectedEvent { player: _ } = this
                            .lock()
                            .unwrap()
                            .trigger_event(ProxyEvent::PlayerDisconnectedEvent {
                                player: player.clone(),
                            })
                        else {
                            return Ok(());
                        };
                    }
                }
            }
        }

        Ok(())
    }

    pub fn start(self) {
        let listener = TcpListener::bind(&self.config.host).expect("invalid host");

        info!("meexprox started on {}", &self.config.host);

        let mutex_self = Arc::new(Mutex::new(self));

        for client in listener.incoming() {
            if let Ok(client) = client {
                let mutex_self_clone = mutex_self.clone();
                thread::spawn(move || {
                    match Self::accept_client(mutex_self_clone, client) {
                        Ok(_) => {}
                        Err(_) => {
                            // error!("connection error: {:?}", e);
                        }
                    };
                });
            }
        }
    }
}

pub type PlayerMutex = Arc<Mutex<ProxyPlayer>>;
pub type MeexProxMutex = Arc<Mutex<MeexProx>>;
