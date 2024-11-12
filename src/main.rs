use std::{fs::{self, File}, path::Path};

use log::LevelFilter;
use meexprox::{config::ProxyConfig, MeexProx};
use simplelog::{ColorChoice, CombinedLogger, Config, TermLogger, TerminalMode, WriteLogger};



// pub struct MyEventListener {}

// impl EventListener for MyEventListener {

// }

pub fn main() {
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

    let config_path = Path::new("config.yml");

    if !config_path.exists() {
        fs::write(config_path, include_bytes!("../config.yml"))
            .expect("config write error");
    }

    let config = ProxyConfig::load(config_path).expect("config parse error");

    let meexprox = MeexProx::new(config);
    // meexprox.add_event_listener(Box::new(MyEventListener {}));
    meexprox.start();
}
