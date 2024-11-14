use rust_mc_proto::ProtocolError;

#[derive(Debug)]
pub enum ProxyError {
    ConfigParse,
    ServerConnect,
    EventChanged,
    HandshakePacket,
    LoginPacket,
    PeerAddr,
    ProtocolError(ProtocolError),
    ConnectionClosed
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({:?})", self)
    }
}

impl std::error::Error for ProxyError {}

pub trait AsProxyError {
    fn as_proxy(self) -> ProxyError;
}

pub trait AsProxyResult<T> {
    fn as_proxy(self) -> Result<T, ProxyError>;
}

impl AsProxyError for ProtocolError {
    fn as_proxy(self) -> ProxyError {
        match self {
            Self::ConnectionClosedError => {
                ProxyError::ConnectionClosed
            }, i => {
                ProxyError::ProtocolError(i)
            }
        }
    }
}

impl <T> AsProxyResult<T> for Result<T, ProtocolError> {
    fn as_proxy(self) -> Result<T, ProxyError> {
        self.map_err(|o| o.as_proxy())
    }
}