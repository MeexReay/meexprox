use log::debug;
use meexprox::{
    EventListener, MeexProx, MeexProxMutex, ProxyConfig,
    ProxyEvent::{self, *},
    ProxyPlayer,
};
use rust_mc_proto::DataBufferReader;
use simplelog::{
    ColorChoice, CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode, WriteLogger,
};
use std::{error::Error, fs::File, sync::atomic::Ordering};

pub struct MyEventListener {}

impl EventListener for MyEventListener {
    fn on_event(
        &mut self,
        this: MeexProxMutex,
        event: &mut ProxyEvent,
    ) -> Result<(), Box<dyn Error>> {
        match event {
            RecvServerPacketEvent { packet, player } => {
                // debug!("recv server packet event");
            }
            SendServerPacketEvent {
                packet,
                player,
                cancel,
            } => {
                // debug!("send server packet event");
            }
            SendClientPacketEvent {
                packet,
                player,
                cancel,
            } => {
                // debug!("send client packet event");
            }
            RecvClientPacketEvent { packet, player } => {
                // debug!("recv client packet event");

                if packet.id() == 0x03 || packet.id() == 0x04 {
                    let command = packet.read_string()?;

                    if command == "reconnect" {
                        println!("reconnect wow");
                        ProxyPlayer::reconnect(player.clone(), this.clone(), "localhost", 25565)
                            .unwrap();
                    }
                }
            }
            PlayerConnectedEvent { player } => {
                debug!("player connected");
            }
            PlayerConnectingServerEvent {
                player,
                server,
                cancel,
            } => {
                debug!("player connecting server");
            }
            PlayerConnectingIPEvent { player, ip, cancel } => {
                debug!("player connecting ip");
            }
            PlayerDisconnectedEvent { player } => {
                debug!("player disconnected");
            }
            StatusRequestEvent {
                status,
                client_address,
                server_address,
                server_port,
                cancel,
            } => {
                debug!("status request");
                *status = String::from("123123");
            }
        }

        Ok(())
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
