use super::{MeexProx, MeexProxMutex, PlayerMutex, ProxyServer};
use rust_mc_proto::Packet;
use std::{
    error::Error,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
};

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
        cancel: AtomicBool,
    },

    /// client <- proxy <- server \
    /// &nbsp;      | \
    /// &nbsp;      SendClientPacketEvent
    SendClientPacketEvent {
        packet: Packet,
        player: PlayerMutex,
        cancel: AtomicBool,
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

    PlayerDisconnectedEvent {
        player: PlayerMutex,
    },

    PlayerConnectingServerEvent {
        player: PlayerMutex,
        server: ProxyServer,
        cancel: AtomicBool,
    },

    PlayerConnectingIPEvent {
        player: PlayerMutex,
        ip: String,
        cancel: AtomicBool,
    },

    StatusRequestEvent {
        status: String,
        client_address: SocketAddr,
        server_address: String,
        server_port: u16,
        cancel: AtomicBool,
    },
}

impl ProxyEvent {
    pub fn status_request(
        meexprox: MeexProxMutex,
        status: String,
        client_address: SocketAddr,
        server_address: String,
        server_port: u16,
    ) -> (String, bool) {
        let ProxyEvent::StatusRequestEvent {
            status,
            client_address: _,
            server_address: _,
            server_port: _,
            cancel,
        } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::StatusRequestEvent {
                status: status.clone(),
                client_address,
                server_address,
                server_port,
                cancel: AtomicBool::from(false),
            },
        )
        else {
            return (status, false);
        };
        (status, cancel.load(Ordering::Relaxed))
    }

    pub fn player_connecting_ip(
        meexprox: MeexProxMutex,
        player: PlayerMutex,
        ip: String,
    ) -> (String, bool) {
        let ProxyEvent::PlayerConnectingIPEvent {
            ip,
            player: _,
            cancel,
        } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::PlayerConnectingIPEvent {
                ip: ip.clone(),
                player,
                cancel: AtomicBool::from(false),
            },
        )
        else {
            return (ip, false);
        };
        (ip, cancel.load(Ordering::Relaxed))
    }

    pub fn player_connecting_server(
        meexprox: MeexProxMutex,
        player: PlayerMutex,
        server: ProxyServer,
    ) -> (ProxyServer, bool) {
        let ProxyEvent::PlayerConnectingServerEvent {
            server,
            player: _,
            cancel,
        } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::PlayerConnectingServerEvent {
                server: server.clone(),
                player,
                cancel: AtomicBool::from(false),
            },
        )
        else {
            return (server, false);
        };
        (server, cancel.load(Ordering::Relaxed))
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
    ) -> (Packet, bool) {
        let ProxyEvent::SendClientPacketEvent {
            packet,
            player: _,
            cancel,
        } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::SendClientPacketEvent {
                packet: packet.clone(),
                player,
                cancel: AtomicBool::from(false),
            },
        )
        else {
            return (packet, false);
        };
        (packet, cancel.load(Ordering::Relaxed))
    }

    pub fn send_server_packet(
        meexprox: MeexProxMutex,
        packet: Packet,
        player: PlayerMutex,
    ) -> (Packet, bool) {
        let ProxyEvent::SendServerPacketEvent {
            packet,
            player: _,
            cancel,
        } = MeexProx::trigger_event(
            meexprox,
            ProxyEvent::SendServerPacketEvent {
                packet: packet.clone(),
                player,
                cancel: AtomicBool::from(false),
            },
        )
        else {
            return (packet, false);
        };
        (packet, cancel.load(Ordering::Relaxed))
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
