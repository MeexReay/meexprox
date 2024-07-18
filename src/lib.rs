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
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
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
    connection_id: Arc<AtomicUsize>,
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
        connection_id: Arc<AtomicUsize>,
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
            connection_id,
        }
    }

    pub fn client_conn(&self) -> &MinecraftConnection<TcpStream> {
        &self.client_conn
    }

    pub fn server_conn(&self) -> &MinecraftConnection<TcpStream> {
        &self.client_conn
    }

    pub fn client_conn_mut(&mut self) -> &mut MinecraftConnection<TcpStream> {
        &mut self.client_conn
    }

    pub fn server_conn_mut(&mut self) -> &mut MinecraftConnection<TcpStream> {
        &mut self.client_conn
    }

    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    pub fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn server(&self) -> Option<&ProxyServer> {
        self.server.as_ref()
    }

    pub fn shared_secret(&self) -> Option<&Vec<u8>> {
        self.shared_secret.as_ref()
    }

    pub fn verify_token(&self) -> Option<&Vec<u8>> {
        self.verify_token.as_ref()
    }

    pub fn connection_id(&self) -> Arc<AtomicUsize> {
        self.connection_id.clone()
    }

    pub fn connect_to_ip(
        player: PlayerMutex,
        this: MeexProxMutex,
        ip: &str,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        todo!()
    }

    pub fn connect_to_server(
        player: PlayerMutex,
        this: MeexProxMutex,
        server: ProxyServer,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        todo!()
    }

    fn send_handshake(
        this: PlayerMutex,
        meexprox: MeexProxMutex,
        player_forwarding: PlayerForwarding,
        addr: SocketAddr,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), ProtocolError> {
        let protocol_version = this.lock().unwrap().protocol_version;

        let packet = Packet::build(0x00, move |packet| {
            packet.write_u16_varint(protocol_version)?;
            packet.write_string(&server_address)?;
            packet.write_unsigned_short(server_port)?;
            packet.write_u8_varint(2)?;

            if let PlayerForwarding::Handshake = player_forwarding {
                if let SocketAddr::V4(addr) = addr {
                    packet.write_boolean(false)?; // is ipv6
                    packet.write_unsigned_short(addr.port())?; // port
                    packet.write_bytes(&addr.ip().octets())?; // octets
                } else if let SocketAddr::V6(addr) = addr {
                    packet.write_boolean(true)?;
                    packet.write_unsigned_short(addr.port())?;
                    packet.write_bytes(&addr.ip().octets())?;
                }
            }

            Ok(())
        })?;

        let packet = ProxyEvent::send_server_packet(meexprox, packet, this.clone());

        this.lock().unwrap().server_conn.write_packet(&packet)?;

        Ok(())
    }

    fn send_login(this: PlayerMutex, meexprox: MeexProxMutex) -> Result<(), ProtocolError> {
        if let Some(player_name) = this.lock().unwrap().name.as_ref() {
            if let Some(player_uuid) = this.lock().unwrap().uuid.as_ref() {
                let packet = Packet::build(0x00, move |packet| {
                    packet.write_string(&player_name)?;
                    packet.write_uuid(&player_uuid)?;
                    Ok(())
                })?;

                let packet = ProxyEvent::send_server_packet(meexprox, packet, this.clone());

                this.lock().unwrap().server_conn.write_packet(&packet)?;
            }
        }

        Ok(())
    }

    fn connect(
        this: PlayerMutex,
        meexprox: MeexProxMutex,
        player_forwarding: PlayerForwarding,
        server_address: &str,
        server_port: u16,
        logged: bool,
    ) -> Result<(), Box<dyn Error>> {
        let mut client_conn = this.lock().unwrap().client_conn.try_clone().unwrap();
        let mut server_conn = this.lock().unwrap().server_conn.try_clone().unwrap();

        let server = this.lock().unwrap().server.clone();

        let addr = client_conn.get_ref().peer_addr().unwrap();
        let Some(name) = this.lock().unwrap().name.clone() else {
            return Ok(());
        };
        let server_config = meexprox.lock().unwrap().config.clone();

        let atomic_connection_id = this.lock().unwrap().connection_id.clone();
        let connection_id = this.lock().unwrap().connection_id.load(Ordering::Relaxed);

        if !logged {
            ProxyPlayer::send_handshake(
                this.clone(),
                meexprox.clone(),
                player_forwarding,
                addr,
                server_address,
                server_port,
            )?;

            ProxyPlayer::send_login(this.clone(), meexprox.clone())?;

            while let Ok(mut packet) = server_conn.read_packet() {
                if packet.id() == 0x01 {
                    if let Some(shared_secret) = this.lock().unwrap().shared_secret.clone() {
                        if let Some(verify_token) = this.lock().unwrap().verify_token.clone() {
                            let mut enc_response = Packet::empty(0x01);

                            enc_response.write_usize_varint(shared_secret.len())?;
                            enc_response.write_bytes(&shared_secret)?;
                            enc_response.write_usize_varint(shared_secret.len())?;
                            enc_response.write_bytes(&verify_token)?;

                            let enc_response = ProxyEvent::send_server_packet(
                                meexprox.clone(),
                                enc_response,
                                this.clone(),
                            );

                            server_conn.write_packet(&enc_response)?;
                        }
                    }
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
                }

                if packet.id() == 0x02 {
                    break;
                }
            }

            let login_ack = Packet::empty(0x03);

            let login_ack =
                ProxyEvent::send_server_packet(meexprox.clone(), login_ack, this.clone());

            server_conn.write_packet(&login_ack)?;
        }

        thread::spawn({
            let mut client_conn = client_conn.try_clone().unwrap();
            let mut server_conn = server_conn.try_clone().unwrap();

            let this = this.clone();
            let meexprox = meexprox.clone();
            let name = name.clone();
            let atomic_connection_id = atomic_connection_id.clone();

            move || {
                let _ = || -> Result<(), ProtocolError> {
                    while atomic_connection_id.load(Ordering::Relaxed) == connection_id {
                        let packet = match client_conn.read_packet() {
                            Ok(packet) => packet,
                            Err(_) => break,
                        };

                        let packet =
                            ProxyEvent::recv_client_packet(meexprox.clone(), packet, this.clone());

                        let packet =
                            ProxyEvent::send_server_packet(meexprox.clone(), packet, this.clone());

                        server_conn.write_packet(&packet)?;
                    }

                    Ok(())
                }();

                if atomic_connection_id.load(Ordering::Relaxed) == connection_id {
                    if meexprox.lock().unwrap().remove_player(this.clone()) {
                        info!("{} disconnected player {}", addr.to_string(), name);
                        ProxyEvent::player_disconnected(meexprox.clone(), this.clone());
                    }
                }
            }
        });

        let _ = || -> Result<(), ProtocolError> {
            while atomic_connection_id.load(Ordering::Relaxed) == connection_id {
                let packet = match server_conn.read_packet() {
                    Ok(packet) => packet,
                    Err(_) => break,
                };

                let packet = ProxyEvent::recv_server_packet(meexprox.clone(), packet, this.clone());

                let packet = ProxyEvent::send_client_packet(meexprox.clone(), packet, this.clone());

                client_conn.write_packet(&packet)?;
            }

            Ok(())
        }();

        if atomic_connection_id.load(Ordering::Relaxed) == connection_id {
            if meexprox.lock().unwrap().remove_player(this.clone()) {
                info!("{} disconnected player {}", addr.to_string(), name);
                ProxyEvent::player_disconnected(meexprox.clone(), this.clone());
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum ProxyEvent {
    /// client <- proxy <- server \
    /// &nbsp;               | \
    /// &nbsp;               RecvServerPacketEvent
    RecvServerPacketEvent {
        packet: Packet,
        player: PlayerMutex,
    },

    /// client -> proxy -> server \
    /// &nbsp;               | \
    /// &nbsp;               SendServerPacketEvent
    SendServerPacketEvent {
        packet: Packet,
        player: PlayerMutex,
    },

    /// client <- proxy <- server \
    /// &nbsp;      | \
    /// &nbsp;      SendClientPacketEvent
    SendClientPacketEvent {
        packet: Packet,
        player: PlayerMutex,
    },

    /// client -> proxy -> server \
    /// &nbsp;      | \
    /// &nbsp;      RecvClientPacketEvent
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

impl ProxyEvent {
    pub fn status_request(
        meexprox: MeexProxMutex,
        status: String,
        client_address: SocketAddr,
        server_address: String,
        server_port: u16,
    ) -> String {
        let ProxyEvent::StatusRequestEvent {
            status,
            client_address: _,
            server_address: _,
            server_port: _,
        } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::StatusRequestEvent {
                status: status.clone(),
                client_address,
                server_address,
                server_port,
            },
        )
        else {
            return status;
        };
        status
    }

    pub fn player_connecting_server(
        meexprox: MeexProxMutex,
        player: PlayerMutex,
        server: ProxyServer,
    ) -> ProxyServer {
        let ProxyEvent::PlayerConnectingServerEvent { server, player: _ } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::PlayerConnectingServerEvent {
                server: server.clone(),
                player,
            },
        ) else {
            return server;
        };
        server
    }

    pub fn player_disconnected(meexprox: MeexProxMutex, player: PlayerMutex) -> () {
        let ProxyEvent::PlayerDisconnectedEvent { player: _ } =
            MeexProx::trigger_event(meexprox, ProxyEvent::PlayerDisconnectedEvent { player })
        else {
            return;
        };
    }

    pub fn player_connected(meexprox: MeexProxMutex, player: PlayerMutex) -> () {
        let ProxyEvent::PlayerConnectedEvent { player: _ } =
            MeexProx::trigger_event(meexprox, ProxyEvent::PlayerConnectedEvent { player })
        else {
            return;
        };
    }

    pub fn send_client_packet(
        meexprox: MeexProxMutex,
        packet: Packet,
        player: PlayerMutex,
    ) -> Packet {
        let ProxyEvent::SendClientPacketEvent { packet, player: _ } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::SendClientPacketEvent {
                packet: packet.clone(),
                player,
            },
        ) else {
            return packet;
        };
        packet
    }

    pub fn send_server_packet(
        meexprox: MeexProxMutex,
        packet: Packet,
        player: PlayerMutex,
    ) -> Packet {
        let ProxyEvent::SendServerPacketEvent { packet, player: _ } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::SendServerPacketEvent {
                packet: packet.clone(),
                player,
            },
        ) else {
            return packet;
        };
        packet
    }

    pub fn recv_server_packet(
        meexprox: MeexProxMutex,
        packet: Packet,
        player: PlayerMutex,
    ) -> Packet {
        let ProxyEvent::RecvServerPacketEvent { packet, player: _ } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::RecvServerPacketEvent {
                packet: packet.clone(),
                player,
            },
        ) else {
            return packet;
        };
        packet
    }

    pub fn recv_client_packet(
        meexprox: MeexProxMutex,
        packet: Packet,
        player: PlayerMutex,
    ) -> Packet {
        let ProxyEvent::RecvClientPacketEvent { packet, player: _ } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::RecvClientPacketEvent {
                packet: packet.clone(),
                player,
            },
        ) else {
            return packet;
        };
        packet
    }
}

pub trait EventListener {
    fn on_event(
        &mut self,
        meexprox: MeexProxMutex,
        event: &mut ProxyEvent,
    ) -> Result<(), Box<dyn Error>>;
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

    pub fn trigger_event(this: MeexProxMutex, mut event: ProxyEvent) -> ProxyEvent {
        for event_listener in &mut this.lock().unwrap().event_listeners {
            let _ = event_listener.on_event(this.clone(), &mut event);
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
                    } = MeexProx::trigger_event(
                        this.clone(),
                        ProxyEvent::StatusRequestEvent {
                            status: server_status.clone(),
                            client_address: addr.clone(),
                            server_address: server_address.clone(),
                            server_port,
                        },
                    )
                    else {
                        return Ok(());
                    };

                    server_packet = Packet::build(0x00, |p| p.write_string(&server_status))?;
                }

                client_conn.write_packet(&server_packet)?;
            }
        } else if next_state == 2 {
            let player = Arc::new(Mutex::new(ProxyPlayer::new(
                client_conn.try_clone().unwrap(),
                server_conn.try_clone().unwrap(),
                None,
                None,
                protocol_version,
                Some(server.clone()),
                None,
                None,
                Arc::new(AtomicUsize::new(0)),
            )));

            this.lock().unwrap().players.push(player.clone());

            let mut login_start = client_conn.read_packet()?;

            player.lock().unwrap().name = Some(login_start.read_string()?);
            player.lock().unwrap().uuid = Some(login_start.read_uuid()?);

            server_conn.write_packet(&login_start)?;

            while let Ok(mut packet) = server_conn.read_packet() {
                client_conn.write_packet(&packet)?;

                if packet.id() == 0x01 {
                    let mut enc_response = client_conn.read_packet()?;

                    let shared_secret_length = enc_response.read_usize_varint()?;
                    player.lock().unwrap().shared_secret =
                        Some(enc_response.read_bytes(shared_secret_length)?);
                    let verify_token_length = enc_response.read_usize_varint()?;
                    player.lock().unwrap().verify_token =
                        Some(enc_response.read_bytes(verify_token_length)?);

                    server_conn.write_packet(&enc_response)?;
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
                }

                if packet.id() == 0x02 {
                    break;
                }
            }

            // println!("lac re");
            // let login_ack = client_conn.read_packet()?;
            // println!("lac {}", login_ack.id());
            // if login_ack.id() != 0x03 {
            //     return Ok(());
            // }

            thread::spawn({
                let this = this.clone();

                move || {
                    info!(
                        "{} connected player {}",
                        addr.to_string(),
                        player.lock().unwrap().name.clone().unwrap()
                    );
                    ProxyEvent::player_connected(this.clone(), player.clone());

                    let _ = ProxyPlayer::connect(
                        player,
                        this,
                        server_config.player_forwarding,
                        &server_address,
                        server_port,
                        true,
                    );
                }
            });
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
