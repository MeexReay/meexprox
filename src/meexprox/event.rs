use std::{any::Any, net::SocketAddr};

use make_event::MakeEvent;

use super::error::ProxyError;

pub trait Event {
    fn name(&self) -> String;
    fn is_cancelled(&self) -> bool;
    fn cancel(&mut self);
}

pub trait AsAny {
    fn as_any_ref(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn as_any_box(self: Box<Self>) -> Box<dyn Any>;
}

impl<T> AsAny for T
where
    T: Any,
{
    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any_box(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

pub trait EventListener<T: Event>: AsAny {
    fn on_event(&self, event: &mut T) -> Result<(), ProxyError>;
}

#[derive(MakeEvent)]
#[MakeEvent("status")]
pub struct StatusEvent {
    cancelled: bool,
    addr: SocketAddr,
    #[setter]
    motd: String,
    server_address: String,
    server_port: u16,
    protocol_version: u16
}