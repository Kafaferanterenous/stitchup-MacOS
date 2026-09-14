//! EngineError — structured errors for the Stitchup core (STAGE0_ENGINE_INTERFACES §1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    OpenFailed,
    Encrypted,
    PasswordRequired,
    Malformed,
    Unsupported,
    PermissionDenied,
    IoError,
    Cancelled,
    AtomicPublishFailed,
    ValidationFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineError {
    pub code: ErrorCode,
    pub message: String,
}

impl EngineError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::new(ErrorCode::IoError, e.to_string())
    }
}

impl From<lopdf::Error> for EngineError {
    fn from(e: lopdf::Error) -> Self {
        EngineError::new(ErrorCode::Malformed, e.to_string())
    }
}

#[macro_export]
macro_rules! err {
    ($code:expr, $($arg:tt)*) => {
        $crate::EngineError::new($code, format!($($arg)*))
    };
}