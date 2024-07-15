use meexprox::{MeexProx, ProxyConfig};
use simplelog::{CombinedLogger, TermLogger, Config, LevelFilter, TerminalMode, ColorChoice, WriteLogger};
use std::fs::File;

fn main() {
    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
            WriteLogger::new(LevelFilter::Info, Config::default(), File::create("latest.log").unwrap()),
        ]
    ).unwrap();

    let config = ProxyConfig::load("config.yml").expect("config parse error");
    let meexprox = MeexProx::new(config);
    meexprox.start();
}
