use std::fmt;

/// Application error. Secrets are never included in Display output.
#[derive(Debug)]
pub enum Error {
    Config(String),
    Stun(String),
    Api(String),
    Http(String),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "config: {msg}"),
            Self::Stun(msg) => write!(f, "stun: {msg}"),
            Self::Api(msg) => write!(f, "cloudflare: {msg}"),
            Self::Http(msg) => write!(f, "http: {msg}"),
            Self::Io(err) => write!(f, "io: {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ureq::Error> for Error {
    fn from(value: ureq::Error) -> Self {
        Self::Http(value.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Http(format!("json: {value}"))
    }
}
