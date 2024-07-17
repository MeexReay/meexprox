use log::debug;
use meexprox::{EventListener, MeexProx, ProxyConfig, ProxyEvent, ProxyEvent::*};
use simplelog::{
    ColorChoice, CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode, WriteLogger,
};
use std::fs::File;

pub struct MyEventListener {}

impl EventListener for MyEventListener {
    fn on_event(&mut self, event: &mut ProxyEvent) {
        match event {
            RecvServerPacketEvent { packet, player } => {
                // debug!("recv server packet event");
            }
            SendServerPacketEvent { packet, player } => {
                // debug!("send server packet event");
            }
            SendClientPacketEvent { packet, player } => {
                // debug!("send client packet event");
            }
            RecvClientPacketEvent { packet, player } => {
                // debug!("recv client packet event");
            }
            PlayerConnectedEvent { player } => {
                debug!("player connected event");
            }
            PlayerConnectingServerEvent { player, server } => {
                debug!("player connecting server event");
            }
            PlayerConnectingIPEvent { player, ip } => {
                debug!("player connecting ip event");
            }
            PlayerDisconnectedEvent { player } => {
                debug!("player disconnected event");
            }
            StatusRequestEvent {
                status,
                client_address,
                server_address,
                server_port,
            } => {
                debug!("status request event");
            }
        }
    }
}

fn main() {
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Debug,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            File::create("latest.log").unwrap(),
        ),
    ])
    .unwrap();

    let config = ProxyConfig::load("config.yml").expect("config parse error");

    let mut meexprox = MeexProx::new(config);

    meexprox.add_event_listener(Box::new(MyEventListener {}));

    meexprox.start();
}
