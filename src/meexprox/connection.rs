use std::{net::{SocketAddr, TcpStream}, sync::{Arc, Mutex}, thread};

use bytebuffer::ByteBuffer;
use ignore_result::Ignore;
use log::info;
use ring::hmac;
use rust_mc_proto::{DataBufferReader, DataBufferWriter, MCConnTcp, Packet, ProtocolError};
use uuid::Uuid;

use super::{config::{PlayerForwarding, ProxyConfig, ServerInfo}, error::{AsProxyResult, ProxyError}};

#[derive(Clone, Debug)]
pub struct LoginInfo {
    protocol_version: u16,
    server_address: String,
    server_port: u16,
    name: String,
    uuid: Uuid,
    shared_secret: Option<Vec<u8>>,
    verify_token: Option<Vec<u8>>
}

impl LoginInfo {
    pub fn write(&self, _config: &ProxyConfig, stream: &mut MCConnTcp) -> Result<(), ProtocolError> {
        stream.write_packet(&Packet::build(0x00, |p| {
            p.write_u16_varint(self.protocol_version)?;
            p.write_string(&self.server_address)?;
            p.write_short(self.server_port as i16)?;
            p.write_u8_varint(2)
        })?)?;

        stream.write_packet(&Packet::build(0x00, |p| {
            p.write_string(&self.name)?;
            p.write_uuid(&self.uuid)
        })?)?;

        loop {
            let mut packet = stream.read_packet()?;

            match packet.id() {
                0x01 => {
                    stream.write_packet(&Packet::build(0x00, |p| {
                        p.write_usize_varint(self.shared_secret.as_ref().unwrap().len())?;
                        p.write_bytes(&self.shared_secret.as_ref().unwrap())?;
                        p.write_usize_varint(self.verify_token.as_ref().unwrap().len())?;
                        p.write_bytes(&self.verify_token.as_ref().unwrap())
                    })?)?;
                }
                0x02 => {
                    break;
                }
                0x03 => {
                    let compression = Some(packet.read_usize_varint()?);
                    stream.set_compression(compression);
                }
                _ => {}
            }
        }

        stream.write_packet(&Packet::empty(0x03))?;

        Ok(())
    }
}

pub struct Player {
    client_conn: Arc<Mutex<MCConnTcp>>,
    server_conn: Arc<Mutex<MCConnTcp>>,
    login_info: Option<LoginInfo>,
    pub name: String,
    pub uuid: Uuid,
    pub server: Option<ServerInfo>,
    pub protocol_version: u16,
    pub addr: SocketAddr
}

impl Player {
    pub fn read(
        _config: &ProxyConfig,
        protocol_version: u16, 
        server_address: String, 
        server_port: u16, 
        server: ServerInfo,
        addr: SocketAddr,
        mut client_conn: MCConnTcp, 
        mut server_conn: MCConnTcp
    ) -> Result<Player, ProxyError> {
        let mut packet = client_conn.read_packet().as_proxy()?;

        if packet.id() != 0x00 { return Err(ProxyError::LoginPacket); }

        let name = packet.read_string().as_proxy()?;
        let uuid = packet.read_uuid().as_proxy()?;

        server_conn.write_packet(&packet).as_proxy()?;

        let mut player = Player {
            addr,
            client_conn: Arc::new(Mutex::new(client_conn)),
            server_conn: Arc::new(Mutex::new(server_conn)),
            login_info: None,
            name: name.clone(),
            uuid,
            server: Some(server.clone()),
            protocol_version
        };

        let mut shared_secret = None;
        let mut verify_token = None;

        loop {
            let mut packet = player.read_server_packet()?;
            match packet.id() {
                0x01 => {
                    player.write_client_packet(&packet)?;
                    let mut packet = player.read_client_packet()?;
                    let i = packet.read_usize_varint().as_proxy()?;
                    shared_secret = Some(packet.read_bytes(i).as_proxy()?);
                    let i = packet.read_usize_varint().as_proxy()?;
                    verify_token = Some(packet.read_bytes(i).as_proxy()?);
                    player.write_server_packet(&packet)?;
                }
                0x02 => {
                    player.write_client_packet(&packet)?;
                    // player.write_server_packet(&player.read_client_packet()?)?;
                    break;
                }
                0x03 => {
                    player.write_client_packet(&packet)?;
                    let compression = Some(packet.read_usize_varint().as_proxy()?);
                    player.set_server_compression(compression);
                    player.set_client_compression(compression);
                }
                0x04 => { // login plugin request
                    let message_id = packet.read_isize_varint().as_proxy()?;
                    let channel = packet.read_string().as_proxy()?;

                    if channel == "velocity:player_info" {
                        if let PlayerForwarding::Velocity(secret) = &server.player_forwarding {
                            let version: u8 = if packet.buffer().len() - packet.buffer().get_rpos() == 1 {
                                packet.read_byte().as_proxy()?
                            } else {
                                1
                            };

                            let response = Packet::build(0x02, |p| {
                                p.write_isize_varint(message_id)?;
                                p.write_boolean(true)?;

                                let mut buf = ByteBuffer::new();
                                DataBufferWriter::write_u8_varint(&mut buf, version)?;
                                DataBufferWriter::write_string(&mut buf, &addr.to_string())?;
                                DataBufferWriter::write_uuid(&mut buf, &uuid)?;
                                DataBufferWriter::write_string(&mut buf, &name)?;
                                DataBufferWriter::write_u8_varint(&mut buf, 0)?; // properties // maybe fix later
                                let buf = buf.as_bytes();

                                let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
                                let sig = hmac::sign(&key, &buf);

                                p.write_bytes(sig.as_ref())?;
                                p.write_bytes(buf.as_ref())?;

                                Ok(())
                            }).as_proxy()?;

                            player.write_server_packet(&response)?;
                            continue;
                        }
                    }

                    player.write_client_packet(&packet)?;
                    player.write_server_packet(&player.read_client_packet()?)?;
                }
                _ => {
                    return Err(ProxyError::LoginPacket);
                },
            }
        }

        player.login_info = Some(LoginInfo {
            protocol_version,
            server_address,
            server_port,
            name,
            uuid,
            shared_secret,
            verify_token
        });

        player.client_recv_loop();
        player.server_recv_loop();

        Ok(player)
    }

    pub fn client_recv_loop(&self) {
        let mut client: rust_mc_proto::MinecraftConnection<TcpStream> = self.client_conn.clone().lock().unwrap().try_clone().unwrap();
        let server = self.server_conn.clone();
        let name = self.name.clone();

        thread::spawn(move || {
            info!("Player {} connected", name);
            while let Ok(packet) = client.read_packet() {
                while !server.lock().unwrap().is_alive() {}
                server.lock().unwrap().write_packet(&packet).ignore();
            }
            info!("Player {} disconnected", name);
            server.lock().unwrap().close();
        });
    }

    pub fn disconnect(&self) {
        self.client_conn.lock().unwrap().close();
        self.server_conn.lock().unwrap().close();
    }

    pub fn kick(&self, text: String) -> Result<(), ProxyError> {
        self.write_client_packet(&Packet::build(
            0x1D, |p| p.write_string(&text)
        ).as_proxy()?)?;
        self.disconnect();
        Ok(())
    }

    pub fn server_recv_loop(&self) {
        let mut server = self.server_conn.clone().lock().unwrap().try_clone().unwrap();
        let client = self.client_conn.clone();
        let server_name = self.server.as_ref().unwrap().name.clone();
        let name = self.name.clone();

        thread::spawn(move || {
            info!("Server {} connected player {}", server_name, name);
            while let Ok(packet) = server.read_packet() {
                client.lock().unwrap().write_packet(&packet).ignore();
            }
            info!("Server {} disconnected player {}", server_name, name);
        });
    }

    pub fn connect_server(&self, config: &ProxyConfig, server: ServerInfo) -> Result<(), ProxyError> {
        self.server_conn.lock().unwrap().close();
        let mut server_conn = MCConnTcp::connect(&server.host).as_proxy()?;
        if let Some(login_info) = &self.login_info {
            login_info.write(config, &mut server_conn).as_proxy()?;
        }
        *self.server_conn.lock().unwrap() = server_conn;
        self.server_recv_loop();
        Ok(())
    }

    pub fn write_client_packet(&self, packet: &Packet) -> Result<(), ProxyError> {
        self.client_conn.lock().unwrap().write_packet(packet).as_proxy()
    }

    pub fn write_server_packet(&self, packet: &Packet) -> Result<(), ProxyError> {
        self.server_conn.lock().unwrap().write_packet(packet).as_proxy()
    }

    fn read_client_packet(&self) -> Result<Packet, ProxyError> {
        self.client_conn.lock().unwrap().read_packet().as_proxy()
    }

    fn read_server_packet(&self) -> Result<Packet, ProxyError> {
        self.server_conn.lock().unwrap().read_packet().as_proxy()
    }

    fn set_server_compression(&self, threshold: Option<usize>) {
        self.server_conn.lock().unwrap().set_compression(threshold);
    }

    fn set_client_compression(&self, threshold: Option<usize>) {
        self.client_conn.lock().unwrap().set_compression(threshold);
    }

    pub fn server_compression(&self) -> Option<usize> {
        self.server_conn.lock().unwrap().compression()
    }

    pub fn client_compression(&self) -> Option<usize> {
        self.client_conn.lock().unwrap().compression()
    }
}