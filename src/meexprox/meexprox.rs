use super::{EventListener, PlayerForwarding, ProxyConfig, ProxyError, ProxyEvent, ProxyServer};
use derivative::Derivative;
use log::{debug, info};
use no_deadlocks::Mutex;
use rust_mc_proto::{
    DataBufferReader, DataBufferWriter, MinecraftConnection, Packet, ProtocolError, Zigzag,
};
use std::{
    error::Error,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
};
use tokio::task::AbortHandle;
use uuid::Uuid;

#[derive(Derivative)]
#[derivative(Debug)]
pub struct ProxyPlayer {
    #[derivative(Debug = "ignore")]
    client_conn: MinecraftConnection<TcpStream>,
    #[derivative(Debug = "ignore")]
    server_conn: MinecraftConnection<TcpStream>,
    connection_threads: Vec<AbortHandle>,
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
        connection_threads: Vec<AbortHandle>,
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
            connection_threads,
        }
    }

    pub fn client_conn(&self) -> &MinecraftConnection<TcpStream> {
        &self.client_conn
    }

    pub fn server_conn(&self) -> &MinecraftConnection<TcpStream> {
        &self.server_conn
    }

    pub fn client_conn_mut(&mut self) -> &mut MinecraftConnection<TcpStream> {
        &mut self.client_conn
    }

    pub fn server_conn_mut(&mut self) -> &mut MinecraftConnection<TcpStream> {
        &mut self.server_conn
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

    pub fn connection_threads(&mut self) -> &mut Vec<AbortHandle> {
        &mut self.connection_threads
    }

    pub fn connect_to_ip(
        this: PlayerMutex,
        meexprox: MeexProxMutex,
        ip: &str,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        let (ip, cancel) =
            ProxyEvent::player_connecting_ip(meexprox.clone(), this.clone(), ip.to_string());
        let ip = &ip;

        if cancel {
            return Ok(());
        }

        for thread in &mut this.lock().unwrap().connection_threads {
            thread.abort();
        }

        this.lock().unwrap().server_conn.close();
        this.lock().unwrap().server_conn = MinecraftConnection::connect(ip)?;

        thread::spawn({
            let player_forwarding = meexprox.lock().unwrap().config.player_forwarding().clone();
            let server_address = server_address.to_string();

            move || {
                let _ = ProxyPlayer::connect(
                    this,
                    meexprox,
                    player_forwarding,
                    &server_address,
                    server_port,
                    false,
                );
            }
        });

        Ok(())
    }

    pub fn connect_to_server(
        this: PlayerMutex,
        meexprox: MeexProxMutex,
        server: ProxyServer,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        let (server, cancel) =
            ProxyEvent::player_connecting_server(meexprox.clone(), this.clone(), server);

        if cancel {
            return Ok(());
        }

        for thread in &mut this.lock().unwrap().connection_threads {
            thread.abort();
        }
        this.lock().unwrap().server_conn.close();

        this.lock().unwrap().server = Some(server.clone());
        this.lock().unwrap().server_conn = MinecraftConnection::connect(server.host())?;

        thread::spawn({
            let player_forwarding = meexprox.lock().unwrap().config.player_forwarding().clone();
            let server_address = server_address.to_string();

            move || {
                let _ = ProxyPlayer::connect(
                    this,
                    meexprox,
                    player_forwarding,
                    &server_address,
                    server_port,
                    false,
                );
            }
        });

        Ok(())
    }

    pub fn reconnect(
        this: PlayerMutex,
        meexprox: MeexProxMutex,
        server_address: &str,
        server_port: u16,
    ) -> Result<(), Box<dyn Error>> {
        for thread in &mut this.lock().unwrap().connection_threads {
            thread.abort();
        }
        this.lock().unwrap().server_conn.close();

        let server_host = this.lock().unwrap().server().unwrap().host().to_string();
        this.lock().unwrap().server_conn = MinecraftConnection::connect(&server_host)?;

        thread::spawn({
            let player_forwarding = meexprox.lock().unwrap().config.player_forwarding().clone();
            let server_address = server_address.to_string();

            move || {
                let _ = ProxyPlayer::connect(
                    this,
                    meexprox,
                    player_forwarding,
                    &server_address,
                    server_port,
                    false,
                );
            }
        });

        Ok(())
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

        let (packet, cancel) = ProxyEvent::send_server_packet(meexprox, packet, this.clone());

        if !cancel {
            this.lock().unwrap().server_conn.write_packet(&packet)?;
        }

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

                let (packet, cancel) =
                    ProxyEvent::send_server_packet(meexprox, packet, this.clone());

                if !cancel {
                    this.lock().unwrap().server_conn.write_packet(&packet)?;
                }
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

        let addr = client_conn.get_ref().peer_addr().unwrap();
        let Some(name) = this.lock().unwrap().name.clone() else {
            return Ok(());
        };

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

                            let (enc_response, cancel) = ProxyEvent::send_server_packet(
                                meexprox.clone(),
                                enc_response,
                                this.clone(),
                            );

                            if !cancel {
                                server_conn.write_packet(&enc_response)?;
                            }
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

            let (login_ack, cancel) =
                ProxyEvent::send_server_packet(meexprox.clone(), login_ack, this.clone());

            if !cancel {
                server_conn.write_packet(&login_ack)?;
            }
        }

        let mut handles = Vec::new();

        handles.push(
            tokio::spawn({
                let mut client_conn = client_conn.try_clone().unwrap();
                let mut server_conn = server_conn.try_clone().unwrap();

                let this = this.clone();
                let meexprox = meexprox.clone();
                let name = name.clone();
                let addr = addr.clone();

                async move {
                    while let Ok(packet) = client_conn.read_packet() {
                        let packet =
                            ProxyEvent::recv_client_packet(meexprox.clone(), packet, this.clone());

                        let (packet, cancel) =
                            ProxyEvent::send_server_packet(meexprox.clone(), packet, this.clone());

                        if !cancel {
                            match server_conn.write_packet(&packet) {
                                Ok(_) => {}
                                Err(_) => {
                                    break;
                                }
                            };
                        }
                    }

                    if meexprox.lock().unwrap().remove_player(this.clone()) {
                        info!("{} disconnected player {}", addr.to_string(), name);
                        ProxyEvent::player_disconnected(meexprox.clone(), this.clone());
                    }
                }
            })
            .abort_handle(),
        );

        handles.push(
            tokio::spawn({
                let this = this.clone();

                async move {
                    while let Ok(packet) = server_conn.read_packet() {
                        let packet =
                            ProxyEvent::recv_server_packet(meexprox.clone(), packet, this.clone());

                        let (packet, cancel) =
                            ProxyEvent::send_client_packet(meexprox.clone(), packet, this.clone());

                        if !cancel {
                            match client_conn.write_packet(&packet) {
                                Ok(_) => {}
                                Err(_) => {
                                    break;
                                }
                            };
                        }
                    }

                    if meexprox.lock().unwrap().remove_player(this.clone()) {
                        info!("{} disconnected player {}", addr.to_string(), name);
                        ProxyEvent::player_disconnected(meexprox.clone(), this.clone());
                    }
                }
            })
            .abort_handle(),
        );

        this.lock().unwrap().connection_threads = handles;

        Ok(())
    }
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
            .or(server_config.default_server().cloned())
            .ok_or(ProxyError::ConfigParse)?;

        let mut server_conn = MinecraftConnection::connect(&server.host())?;

        let handshake = Packet::build(0x00, |handshake| {
            handshake.write_u16_varint(protocol_version)?;
            handshake.write_string(&server_address)?;
            handshake.write_unsigned_short(server_port)?;
            handshake.write_u8_varint(next_state)?;

            if let PlayerForwarding::Handshake = server_config.player_forwarding() {
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

                    let (status, cancel) = ProxyEvent::status_request(
                        this.clone(),
                        server_status.clone(),
                        addr.clone(),
                        server_address.clone(),
                        server_port,
                    );

                    if cancel {
                        break;
                    }

                    server_packet = Packet::build(0x00, |p| p.write_string(&status))?;
                }

                client_conn.write_packet(&server_packet)?;
            }
        } else if next_state == 2 {
            let player = Arc::new(Mutex::new(ProxyPlayer::new(
                client_conn.try_clone().unwrap(),
                server_conn.try_clone().unwrap(),
                Vec::new(),
                None,
                None,
                protocol_version,
                Some(server.clone()),
                None,
                None,
            )));

            let (server, cancel) =
                ProxyEvent::player_connecting_server(this.clone(), player.clone(), server.clone());

            if cancel {
                return Ok(());
            }

            player.lock().unwrap().server = Some(server);

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

            let this = this.clone();

            info!(
                "{} connected player {}",
                addr.to_string(),
                player.lock().unwrap().name.clone().unwrap()
            );
            ProxyEvent::player_connected(this.clone(), player.clone());

            let _ = ProxyPlayer::connect(
                player,
                this,
                server_config.player_forwarding().clone(),
                &server_address,
                server_port,
                true,
            );
        }

        Ok(())
    }

    pub fn start(self) {
        let listener = TcpListener::bind(self.config.host()).expect("invalid host");

        info!("meexprox started on {}", self.config.host());

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
