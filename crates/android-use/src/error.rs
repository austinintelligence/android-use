use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AuError>;

#[derive(Debug, Error)]
pub enum AuError {
    #[error("{message}")]
    Code { code: &'static str, message: String },
    #[error("{message}")]
    Protocol { code: String, message: String },
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl AuError {
    pub fn code(code: &'static str, message: impl Into<String>) -> Self {
        Self::Code {
            code,
            message: message.into(),
        }
    }

    pub fn protocol(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Protocol {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn kind(&self) -> &str {
        match self {
            Self::Code { code, .. } => code,
            Self::Protocol { code, .. } => code,
            Self::Io(_) => "E_IO",
            Self::Json(_) => "E_JSON",
        }
    }

    pub fn compact_message(&self) -> String {
        self.to_string().replace(['\r', '\n'], " ")
    }
}

impl From<std::num::ParseIntError> for AuError {
    fn from(value: std::num::ParseIntError) -> Self {
        Self::code("E_ARGS", value.to_string())
    }
}

impl From<std::num::ParseFloatError> for AuError {
    fn from(value: std::num::ParseFloatError) -> Self {
        Self::code("E_ARGS", value.to_string())
    }
}
