use std::net::SocketAddr;

use rust_mc_proto::Packet;

use super::{config::ServerInfo, connection::Player, error::ProxyError};

pub trait EventListener {
    fn on_server_recv_packet(
        &self,
        packet: &mut Packet,
        player: &Player,
    ) -> Result<(), ProxyError>;

    fn on_server_send_packet(
        &self,
        packet: &mut Packet,
        cancel: &mut bool,
        player: &Player,
    ) -> Result<(), ProxyError>;

    fn on_client_send_packet(
        &self,
        packet: &mut Packet,
        cancel: &mut bool,
        player: &Player,
    ) -> Result<(), ProxyError>;

    fn on_client_recv_packet(
        &self,
        packet: &mut Packet,
        player: &Player,
    ) -> Result<(), ProxyError>;

    fn on_player_connected(
        &self,
        player: &Player,
    ) -> Result<(), ProxyError>;

    fn on_player_disconnected(
        &self,
        player: &Player,
    ) -> Result<(), ProxyError>;

    fn on_player_connecting_server(
        &self,
        player: &Player,
        cancel: &mut bool,
        server: &mut ServerInfo
    ) -> Result<(), ProxyError>;

    fn on_status_request(
        &self,
        status: String,
        client_address: SocketAddr,
        server_address: String,
        server_port: u16,
        cancel: &mut bool,
    ) -> Result<(), ProxyError>;
}