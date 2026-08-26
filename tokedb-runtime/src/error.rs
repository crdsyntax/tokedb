use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
pub enum RuntimeError {
    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(&'static str),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid name `{name}`: {reason}")]
    InvalidName { name: String, reason: &'static str },
    #[error("io error at `{path}`: {message}")]
    Io { path: String, message: String },
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("command `{command}` is not implemented yet")]
    NotImplemented { command: String },
    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    #[error("container `{id}` not found")]
    ContainerNotFound { id: String },
    #[error("container `{name}` already exists")]
    ContainerAlreadyExists { name: String },
    #[error("container `{id}` is not stopped")]
    ContainerNotStopped { id: String },
    #[error("container `{id}` is not running")]
    ContainerNotRunning { id: String },
    #[error("corrupt state for container `{id}`: {reason}")]
    CorruptState { id: String, reason: String },
    #[error("process error: {0}")]
    Process(String),
    #[error("unsafe path `{path}` inside layer archive")]
    UnsafeLayer { path: String },
    #[error("invalid path `{path}`: {reason}")]
    InvalidPath { path: String, reason: &'static str },
    #[error("layer error: {0}")]
    Layer(String),
    #[error("cgroup write failed for `{file}`: {message}")]
    CgroupWrite { file: String, message: String },
    #[error("volume `{name}` not found")]
    VolumeNotFound { name: String },
    #[error("volume `{name}` is busy (backup lock held)")]
    VolumeBusy { name: String },
    #[error("image `{reference}` not found")]
    ImageNotFound { reference: String },
    #[error("image `{reference}` already exists")]
    ImageAlreadyExists { reference: String },
    #[error("image `{reference}` is in use by one or more containers")]
    ImageInUse { reference: String },
    #[error("invalid reference `{reference}`: {reason}")]
    InvalidReference {
        reference: String,
        reason: &'static str,
    },
    #[error("invalid manifest: {reason}")]
    InvalidManifest { reason: String },
    #[error("corrupt image `{reference}`: {reason}")]
    CorruptImage { reference: String, reason: String },
    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("registry error: {0}")]
    Registry(String),
}

impl From<std::io::Error> for RuntimeError {
    fn from(err: std::io::Error) -> Self {
        RuntimeError::Io {
            path: String::new(),
            message: err.to_string(),
        }
    }
}

#[cfg(target_os = "linux")]
impl From<nix::errno::Errno> for RuntimeError {
    fn from(err: nix::errno::Errno) -> Self {
        RuntimeError::Process(err.to_string())
    }
}

impl From<serde_json::Error> for RuntimeError {
    fn from(err: serde_json::Error) -> Self {
        RuntimeError::Serialize(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_serializes_as_typed_variant() {
        let err = RuntimeError::InvalidConfig("missing data root".into());
        let value = serde_json::to_value(&err).unwrap();
        assert_eq!(value, json!({"InvalidConfig": "missing data root"}));
    }

    #[test]
    fn error_display_is_human_readable() {
        let err = RuntimeError::NotImplemented {
            command: "pull".into(),
        };
        assert_eq!(err.to_string(), "command `pull` is not implemented yet");
    }
}
