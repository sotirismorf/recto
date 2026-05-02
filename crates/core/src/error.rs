use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Image(#[from] image::ImageError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("no config directory found")]
    NoConfigDir,

    #[error(
        "unsupported project version: file has {found}, this app supports up to {supported}"
    )]
    UnsupportedProjectVersion { found: u32, supported: u32 },

    #[error("PDF export is not supported via run_batch; use export_to_pdf instead")]
    PdfNotSupportedInBatch,
}
