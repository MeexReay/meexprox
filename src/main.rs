use meexprox::{EventListener, MeexProx, ProxyConfig, ProxyEvent};
use simplelog::{
    ColorChoice, CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode, WriteLogger,
};
use std::fs::File;

pub struct MyEventListener {}

impl EventListener for MyEventListener {
    fn on_event(&mut self, event: &mut ProxyEvent) {
        dbg!(event);
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
