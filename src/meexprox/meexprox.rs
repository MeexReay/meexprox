use log::{error, info};
use rust_mc_proto::{
    read_packet, write_packet, DataBufferReader, DataBufferWriter, MCConnTcp, Packet
};
use std::{
    net::{TcpListener, TcpStream},
    sync::{
        Arc, RwLock, RwLockReadGuard
    }, thread,
};

use super::{config::ProxyConfig, connection::Player, error::{AsProxyResult, ProxyError}, event::{Event, EventListener}};


pub struct MeexProx {
    config: ProxyConfig,
    players: RwLock<Vec<Player>>,
    event_listeners: Vec<Box<dyn EventListener<dyn Event> + Send + Sync>>
}

impl MeexProx {
    pub fn new(config: ProxyConfig) -> MeexProx {
        MeexProx {
            config,
            players: RwLock::new(Vec::new()),
            event_listeners: Vec::new(),
        }
    }

    pub fn add_event_listener(
        &mut self,
        event_listener: Box<dyn EventListener<dyn Event> + Send + Sync>,
    ) {
        self.event_listeners.push(event_listener);
    }

    pub fn trigger_event<T: Event + 'static>(&self, event: &mut T) -> Result<(), ProxyError> { 
        for listener in &self.event_listeners {
            if let Some(listener) = 
                    listener.as_any_ref().downcast_ref::<Box<dyn EventListener<T> + Send + Sync + 'static>>() { 
                listener.on_event(event)?;
            }
        }
        Ok(())
    }

    pub async fn get_players(&self) -> RwLockReadGuard<'_, Vec<Player>> {
        self.players.read().unwrap()
    }

    pub fn accept_client(&self, mut client_conn: TcpStream) -> Result<(), ProxyError> {
        let addr = client_conn.peer_addr().map_err(|_| ProxyError::PeerAddr)?;

        let mut handshake = read_packet(&mut client_conn, None).as_proxy()?;

        if handshake.id() != 0x00 {
            return Err(ProxyError::HandshakePacket);
        }

        let protocol_version = handshake.read_u16_varint().as_proxy()?;
        let server_address = handshake.read_string().as_proxy()?;
        let server_port = handshake.read_unsigned_short().as_proxy()?;
        let next_state = handshake.read_u8_varint().as_proxy()?;

        let server = self.config
            .get_server_by_domain(&server_address)
            .ok_or(ProxyError::ConfigParse)?;

        let mut server_conn = TcpStream::connect(&server.host).map_err(|_| ProxyError::ServerConnect)?;

        let handshake = Packet::build(0x00, |handshake| {
            handshake.write_u16_varint(protocol_version)?;
            handshake.write_string(&server_address)?;
            handshake.write_unsigned_short(server_port)?;
            handshake.write_u8_varint(next_state)?;

            Ok(())
        }).as_proxy()?;

        write_packet(&mut server_conn, None, 0, &handshake).as_proxy()?;

        let mut client_conn = MCConnTcp::new(client_conn);
        let mut server_conn = MCConnTcp::new(server_conn);

        if next_state == 1 {
            loop {
                server_conn.write_packet(&client_conn.read_packet().as_proxy()?).as_proxy()?;
                client_conn.write_packet(&server_conn.read_packet().as_proxy()?).as_proxy()?;
            }
        } else if next_state == 2 {
            self.players.write().unwrap().push(Player::read(
                &self.config,
                protocol_version, 
                server_address, 
                server_port, 
                server, 
                addr,
                client_conn, 
                server_conn
            )?);
        }

        Ok(())
    }

    pub fn start(self) {
        let listener = TcpListener::bind(&self.config.host).expect("invalid host");

        info!("meexprox started on {}", &self.config.host);

        let self_arc = Arc::new(self);

        for client in listener.incoming() {
            if let Ok(client) = client {
                let self_arc = self_arc.clone();
                thread::spawn(move || {
                    match self_arc.accept_client(client) {
                        Ok(_) => {}
                        Err(e) => {
                            error!("connection error: {:?}", e);
                            
                        }
                    };
                });
            }
        }
    }
}