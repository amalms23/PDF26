//! The part of rfd's API PdfCraft uses, for Android, where rfd has no backend.
//!
//! `pdfcraft-ui-egui` depends on this crate under the name `rfd` when it is built for Android, so
//! its pickers (`platform/pickers.rs`) compile and run unchanged. Each dialog becomes a
//! [`Request`] handed to the [`Backend`] the Android app installs with [`set_backend`] (the
//! system's document pickers, through JNI). With no backend every dialog answers "cancelled".
//!
//! Like rfd on macOS, creating the future shows the picker: the request is sent when
//! `pick_file`, `pick_files`, `pick_folder` or `save_file` is called, and the future resolves
//! when the backend replies.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::task::{Context, Poll, Waker};

/// What the picker is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One existing file.
    PickFile,
    /// One or more existing files.
    PickFiles,
    /// A folder.
    PickFolder,
    /// Where to save a new file.
    SaveFile,
}

/// A file type filter: a name and its extensions (without dots; rfd globs such as `[pP][dD][fF]`
/// are passed through as given).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// One picker the app asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    pub title: Option<String>,
    /// The suggested name (Save).
    pub file_name: Option<String>,
    pub filters: Vec<Filter>,
    pub directory: Option<PathBuf>,
}

/// Sends the chosen paths back; an empty list means cancelled.
pub type Reply = Box<dyn FnOnce(Vec<PathBuf>) + Send>;

/// Shows pickers. Installed once by the Android app.
pub trait Backend: Send + Sync {
    /// Show the picker for `request` and call `reply` once with the chosen local paths. A path
    /// returned for [`Kind::SaveFile`] is a file the app may write (and replace by renaming);
    /// the backend takes care of copying it to where the user chose.
    fn show(&self, request: Request, reply: Reply);
}

static BACKEND: OnceLock<Box<dyn Backend>> = OnceLock::new();

/// Install the picker backend. Returns false when one is already installed.
pub fn set_backend(backend: Box<dyn Backend>) -> bool {
    BACKEND.set(backend).is_ok()
}

#[derive(Default)]
struct Slot {
    result: Option<Vec<PathBuf>>,
    waker: Option<Waker>,
}

/// Resolves to the paths the backend replied with.
struct Pending {
    slot: Arc<Mutex<Slot>>,
}

impl Future for Pending {
    type Output = Vec<PathBuf>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Vec<PathBuf>> {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        match slot.result.take() {
            Some(paths) => Poll::Ready(paths),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

fn show(request: Request) -> Pending {
    let slot = Arc::new(Mutex::new(Slot::default()));
    let reply_slot = slot.clone();
    let reply: Reply = Box::new(move |paths| {
        let waker = {
            let mut slot = reply_slot.lock().unwrap_or_else(PoisonError::into_inner);
            slot.result = Some(paths);
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    });
    match BACKEND.get() {
        Some(backend) => backend.show(request, reply),
        None => reply(Vec::new()),
    }
    Pending { slot }
}

/// A chosen file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHandle {
    path: PathBuf,
}

impl FileHandle {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// The file's bytes, or nothing when it can't be read.
    pub async fn read(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap_or_default()
    }
}

impl From<PathBuf> for FileHandle {
    fn from(path: PathBuf) -> Self {
        Self { path }
    }
}

/// rfd's builder for a non-blocking picker.
#[derive(Clone, Debug, Default)]
pub struct AsyncFileDialog {
    title: Option<String>,
    file_name: Option<String>,
    filters: Vec<Filter>,
    directory: Option<PathBuf>,
}

impl AsyncFileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push(Filter { name: name.into(), extensions: extensions.iter().map(ToString::to_string).collect() });
        self
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    pub fn set_directory<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.directory = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn set_can_create_directories(self, _can: bool) -> Self {
        self
    }

    fn request(self, kind: Kind) -> Request {
        Request { kind, title: self.title, file_name: self.file_name, filters: self.filters, directory: self.directory }
    }

    pub fn pick_file(self) -> impl Future<Output = Option<FileHandle>> {
        let pending = show(self.request(Kind::PickFile));
        async move { pending.await.into_iter().next().map(FileHandle::from) }
    }

    pub fn pick_files(self) -> impl Future<Output = Option<Vec<FileHandle>>> {
        let pending = show(self.request(Kind::PickFiles));
        async move {
            let paths = pending.await;
            (!paths.is_empty()).then(|| paths.into_iter().map(FileHandle::from).collect())
        }
    }

    pub fn pick_folder(self) -> impl Future<Output = Option<FileHandle>> {
        let pending = show(self.request(Kind::PickFolder));
        async move { pending.await.into_iter().next().map(FileHandle::from) }
    }

    pub fn save_file(self) -> impl Future<Output = Option<FileHandle>> {
        let pending = show(self.request(Kind::SaveFile));
        async move { pending.await.into_iter().next().map(FileHandle::from) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builder_carries_everything_into_the_request() {
        let request = AsyncFileDialog::new()
            .set_title("Save")
            .set_file_name("a.pdf")
            .add_filter("PDF", &["pdf"])
            .set_directory("/sdcard")
            .request(Kind::SaveFile);
        assert_eq!(request.kind, Kind::SaveFile);
        assert_eq!(request.title.as_deref(), Some("Save"));
        assert_eq!(request.file_name.as_deref(), Some("a.pdf"));
        assert_eq!(request.filters, vec![Filter { name: "PDF".into(), extensions: vec!["pdf".into()] }]);
        assert_eq!(request.directory, Some(PathBuf::from("/sdcard")));
    }

    #[test]
    fn a_reply_before_the_first_poll_still_resolves() {
        let slot = Arc::new(Mutex::new(Slot { result: Some(vec![PathBuf::from("x.pdf")]), waker: None }));
        let got = std::thread::spawn(move || {
            let mut fut = std::pin::pin!(Pending { slot });
            let waker = Waker::noop();
            let mut cx = Context::from_waker(waker);
            fut.as_mut().poll(&mut cx)
        })
        .join();
        assert!(matches!(got, Ok(Poll::Ready(p)) if p == vec![PathBuf::from("x.pdf")]));
    }
}
