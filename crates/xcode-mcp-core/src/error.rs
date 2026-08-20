use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("XCODE_MCP_ROOT not set or invalid: {0}")]
    RootNotConfigured(String),

    #[error("path rejected by security policy: {0}")]
    PathRejected(String),

    #[error("path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("xcodebuild spawn failed: {0}")]
    XcodeSpawnFailed(String),

    #[error("xcodebuild -list failed (exit {exit_code:?}): {stderr_excerpt}")]
    XcodeListFailed {
        exit_code: Option<i32>,
        stderr_excerpt: String,
    },

    #[error("unrecognized -list output format")]
    UnrecognizedListFormat,

    #[error("unrecognized xcresult format")]
    UnrecognizedResultFormat,

    #[error("build not found: {0}")]
    BuildNotFound(String),

    #[error("no Podfile found next to {working_dir}")]
    PodfileNotFound { working_dir: PathBuf },

    #[error("no build available: {hint}")]
    NoBuildAvailable { hint: String },

    #[error("xcresulttool failed (exit {exit_code:?}): {stderr_excerpt}")]
    XcresulttoolFailed {
        exit_code: Option<i32>,
        stderr_excerpt: String,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;
