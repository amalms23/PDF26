//! The part of arboard's API PdfCraft uses, for Android, where arboard has no backend.
//!
//! `pdfcraft-ui-egui` depends on this crate under the name `arboard` when it is built for
//! Android. Text goes through the [`TextBackend`] the Android app installs with
//! [`set_text_backend`] (the system `ClipboardManager`, through JNI); HTML is copied as its plain
//! text alternative. Images and file lists report [`Error::ClipboardNotSupported`], which the
//! app already handles like a clipboard without that format.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Reads and writes the system clipboard's text. Installed once by the Android app.
pub trait TextBackend: Send + Sync {
    /// The clipboard's text, or `None` when it holds none.
    fn get(&self) -> Option<String>;
    /// Put `text` on the clipboard. Returns false when that failed.
    fn set(&self, text: &str) -> bool;
}

static TEXT: OnceLock<Box<dyn TextBackend>> = OnceLock::new();

/// Install the clipboard backend. Returns false when one is already installed.
pub fn set_text_backend(backend: Box<dyn TextBackend>) -> bool {
    TEXT.set(backend).is_ok()
}

/// arboard's error type, with the variants PdfCraft can meet here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    ContentNotAvailable,
    ClipboardNotSupported,
    Unknown { description: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ContentNotAvailable => f.write_str("The clipboard has nothing in the requested format."),
            Error::ClipboardNotSupported => f.write_str("Android's clipboard can't hold this kind of content here."),
            Error::Unknown { description } => write!(f, "Clipboard error: {description}"),
        }
    }
}

impl std::error::Error for Error {}

/// An RGBA image, as in arboard.
#[derive(Debug, Clone)]
pub struct ImageData<'a> {
    pub width: usize,
    pub height: usize,
    pub bytes: Cow<'a, [u8]>,
}

/// The system clipboard.
#[derive(Debug)]
pub struct Clipboard {
    _private: (),
}

fn backend() -> Result<&'static dyn TextBackend, Error> {
    TEXT.get().map(|b| b.as_ref()).ok_or(Error::ClipboardNotSupported)
}

fn put(text: &str) -> Result<(), Error> {
    if backend()?.set(text) { Ok(()) } else { Err(Error::Unknown { description: "the system refused the copy".into() }) }
}

impl Clipboard {
    pub fn new() -> Result<Self, Error> {
        Ok(Self { _private: () })
    }

    pub fn get_text(&mut self) -> Result<String, Error> {
        backend()?.get().ok_or(Error::ContentNotAvailable)
    }

    pub fn set_text<'a, T: Into<Cow<'a, str>>>(&mut self, text: T) -> Result<(), Error> {
        put(&text.into())
    }

    pub fn get_image(&mut self) -> Result<ImageData<'static>, Error> {
        Err(Error::ContentNotAvailable)
    }

    pub fn set_image(&mut self, _image: ImageData) -> Result<(), Error> {
        Err(Error::ClipboardNotSupported)
    }

    pub fn get(&mut self) -> Get<'_> {
        Get { _clipboard: self }
    }

    pub fn set(&mut self) -> Set<'_> {
        Set { _clipboard: self }
    }
}

/// Reads one format.
#[derive(Debug)]
pub struct Get<'clipboard> {
    _clipboard: &'clipboard mut Clipboard,
}

impl Get<'_> {
    pub fn text(self) -> Result<String, Error> {
        backend()?.get().ok_or(Error::ContentNotAvailable)
    }

    pub fn image(self) -> Result<ImageData<'static>, Error> {
        Err(Error::ContentNotAvailable)
    }

    pub fn html(self) -> Result<String, Error> {
        Err(Error::ContentNotAvailable)
    }

    pub fn file_list(self) -> Result<Vec<PathBuf>, Error> {
        Err(Error::ContentNotAvailable)
    }
}

/// Writes one format.
#[derive(Debug)]
pub struct Set<'clipboard> {
    _clipboard: &'clipboard mut Clipboard,
}

impl Set<'_> {
    pub fn text<'a, T: Into<Cow<'a, str>>>(self, text: T) -> Result<(), Error> {
        put(&text.into())
    }

    /// Copies the plain-text alternative; without one there is nothing Android can paste.
    pub fn html<'a, T: Into<Cow<'a, str>>>(self, _html: T, alt_text: Option<T>) -> Result<(), Error> {
        match alt_text {
            Some(text) => put(&text.into()),
            None => Err(Error::ClipboardNotSupported),
        }
    }

    pub fn image(self, _image: ImageData) -> Result<(), Error> {
        Err(Error::ClipboardNotSupported)
    }

    pub fn file_list(self, _file_list: &[impl AsRef<Path>]) -> Result<(), Error> {
        Err(Error::ClipboardNotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_and_files_are_refused_without_a_panic() {
        let mut cb = Clipboard::new().unwrap();
        assert!(cb.get_image().is_err());
        assert!(cb.set_image(ImageData { width: 1, height: 1, bytes: Cow::Borrowed(&[0, 0, 0, 0]) }).is_err());
        assert!(cb.set().file_list(&["a.pdf"]).is_err());
    }
}
