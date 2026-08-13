use std::io;

use serde_json::Value;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AuError>;

#[derive(Debug, Error)]
pub enum AuError {
    #[error("{message}")]
    Code {
        code: &'static str,
        message: String,
        details: Option<Value>,
    },
    #[error("{message}")]
    Protocol {
        code: String,
        message: String,
        details: Option<Value>,
    },
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
            details: None,
        }
    }

    pub fn protocol(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Protocol {
            code: code.into(),
            message: message.into(),
            details: None,
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

    pub fn details(&self) -> Option<&Value> {
        match self {
            Self::Code { details, .. } | Self::Protocol { details, .. } => details.as_ref(),
            Self::Io(_) | Self::Json(_) => None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        match &mut self {
            Self::Code {
                details: current, ..
            }
            | Self::Protocol {
                details: current, ..
            } => *current = Some(details),
            Self::Io(_) | Self::Json(_) => {}
        }
        self
    }

    pub fn with_optional_details(self, details: Option<Value>) -> Self {
        match details {
            Some(details) => self.with_details(details),
            None => self,
        }
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
