//! Framework error type (§44).
//!
//! Recoverable problems are reported as `Result`; the framework never panics on an application
//! error. Unrecoverable problems are limited to genuine programming mistakes.

use std::fmt;

use crate::object::ObjectId;

/// Errors produced by the framework.
#[derive(Debug)]
pub enum UiError {
    /// The windowing system refused to create a window.
    WindowCreation(String),
    /// The platform layer failed to start the event loop.
    EventLoop(String),
    /// No usable GPU adapter was found.
    GpuUnavailable(String),
    /// The GPU device could not be created.
    DeviceCreation(String),
    /// A surface could not be created or configured.
    SurfaceCreation(String),
    /// A GPU resource could not be created.
    ResourceCreation(String),
    /// The requested window does not exist.
    UnknownWindow(u32),
    /// A widget was requested that is not part of the tree.
    UnknownWidget(ObjectId),
    /// The font database could not be initialised.
    FontDatabase(String),
    /// Any other error carrying a message.
    Other(String),
}

impl UiError {
    /// Wraps a `Result` with a context message.
    pub fn context(self, message: &str) -> UiError {
        match self {
            UiError::Other(_) => UiError::Other(message.to_string()),
            other => UiError::Other(format!("{message}: {other}")),
        }
    }
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::WindowCreation(m) => write!(f, "window creation failed: {m}"),
            UiError::EventLoop(m) => write!(f, "event loop failed: {m}"),
            UiError::GpuUnavailable(m) => write!(f, "no suitable GPU adapter: {m}"),
            UiError::DeviceCreation(m) => write!(f, "GPU device creation failed: {m}"),
            UiError::SurfaceCreation(m) => write!(f, "surface creation failed: {m}"),
            UiError::ResourceCreation(m) => write!(f, "GPU resource creation failed: {m}"),
            UiError::UnknownWindow(id) => write!(f, "unknown window {id}"),
            UiError::UnknownWidget(id) => write!(f, "unknown widget {id}"),
            UiError::FontDatabase(m) => write!(f, "font database failed: {m}"),
            UiError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for UiError {}

impl From<std::io::Error> for UiError {
    fn from(value: std::io::Error) -> Self {
        UiError::Other(value.to_string())
    }
}

/// Result alias used across the framework.
pub type Result<T> = std::result::Result<T, UiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        let e = UiError::GpuUnavailable("no vulkan".into());
        assert!(e.to_string().contains("vulkan"));
        assert!(UiError::UnknownWindow(3).to_string().contains('3'));
        assert!(UiError::Other("x".into())
            .context("ctx")
            .to_string()
            .contains("ctx"));
    }
}
