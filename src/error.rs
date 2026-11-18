use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Invalid PPTX package: {0}")]
    InvalidPptx(String),
    #[error("XML part {part} is not well-formed: {source}")]
    Xml {
        part: String,
        #[source]
        source: quick_xml::Error,
    },
    #[error("Invalid XML structure: {0}")]
    XmlTree(#[from] roxmltree::Error),
    #[error("ZIP package error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Could not read settings file {path}: {source}")]
    SettingsJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("Could not write settings file {path}: {source}")]
    SerializeJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("%LOCALAPPDATA% is not available")]
    LocalAppDataUnavailable,
    #[error("Window event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("The GPU renderer could not start: {0}")]
    Gpu(String),
    #[error("Invalid bundled icon asset: {0}")]
    IconAsset(String),
    #[error("Command history invariant failed: {0}")]
    CommandInvariant(&'static str),
    #[error("Invalid command line: {0}")]
    CommandLine(String),
}

impl AppError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
