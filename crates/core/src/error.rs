use thiserror::Error;

/// Convenience alias for the crate's error type.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors emitted by `recto-core`.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Image(#[from] image::ImageError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("no config directory found")]
    NoConfigDir,

    #[error("unsupported project version: file has {found}, this app supports up to {supported}")]
    UnsupportedProjectVersion { found: u32, supported: u32 },

    #[error("export cancelled")]
    Cancelled,
}
