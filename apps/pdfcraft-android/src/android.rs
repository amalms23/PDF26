//! The entry point GameActivity calls, and the Android services behind the app's pickers,
//! clipboard and "Open with".

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use jni::jni_str;
use pdfcraft_ui_egui::{OsEvent, PdfCraftApp};
use winit::platform::android::activity::AndroidApp;

use crate::java::Java;
use crate::picker_request;

/// GameActivity loads `libpdfcraft_android.so` and calls this on its own thread.
// SAFETY: the one symbol android-activity's glue looks up by name; it has the signature it expects.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("pdfcraft"));
    let Some(files) = app.internal_data_path() else {
        log::error!("no internal data folder; PdfCraft can't start");
        return;
    };
    set_environment(&files);
    let java = Java::of(&app);
    if let Some(java) = &java {
        pdfcraft_android_rfd::set_backend(Box::new(Pickers { java: java.clone() }));
        pdfcraft_android_clipboard::set_text_backend(Box::new(Clipboard { java: java.clone() }));
    } else {
        log::warn!("no Java activity: file pickers and the clipboard are unavailable");
    }
    let keyboard = app.clone();
    let cache = files.parent().map_or_else(|| files.join("cache"), |p| p.join("cache"));
    let options = eframe::NativeOptions {
        android_app: Some(app),
        // OpenGL ES runs on every Android device and emulator; Vulkan drivers vary too much.
        renderer: eframe::Renderer::Glow,
        persistence_path: Some(files.join("app.ron")),
        ..Default::default()
    };
    let result = eframe::run_native(
        "PDF26",
        options,
        Box::new(move |cc| {
            let mut app = PdfCraftApp::new();
            app.app_name = "PDF26";
            app.units = pdfcraft_ui_egui::units::Unit::system();
            if let Some(json) = cc.storage.and_then(|s| s.get_string("pdfcraft")) {
                app.restore(&json);
            }
            if let Some(dir) = pdfcraft_ui_egui::RecoveryStore::default_dir() {
                app.enable_recovery(pdfcraft_ui_egui::RecoveryStore::new(dir));
            }
            app.ocr_verify.dictionary_dir = pdfcraft_ui_egui::default_dictionary_dir();
            let shell_java = java.clone();
            if let Some(java) = &shell_java {
                app.share = Some(share_sheet(java.clone(), cache.clone()));
            }
            if let Some(java) = java {
                app.os_events = Some(listen_for_opens(java, cc.egui_ctx.clone()));
            }
            Ok(Box::new(crate::keyboard::Shell::new(app, keyboard, shell_java)))
        }),
    );
    if let Err(e) = result {
        log::error!("PdfCraft stopped: {e}");
    }
}

/// Where the app's folders are: the desktop code finds settings, recovery data and temporary
/// files through these variables, and Android sets none of them (its default temporary folder,
/// /data/local/tmp, can't be written by apps).
fn set_environment(files: &Path) {
    let cache = files.parent().map_or_else(|| files.join("cache"), |p| p.join("cache"));
    let vars: [(&str, PathBuf); 5] = [
        ("HOME", files.to_path_buf()),
        ("XDG_DATA_HOME", files.join("data")),
        ("XDG_CONFIG_HOME", files.join("config")),
        ("XDG_CACHE_HOME", cache.clone()),
        ("TMPDIR", cache),
    ];
    for (key, dir) in vars {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("couldn't create {}: {e}", dir.display());
        }
        // SAFETY: called first thing in `android_main`, before this library starts any thread;
        // the activity's own Java threads never read the C environment.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var(key, &dir);
        }
    }
}

/// Files other apps send ("Open with", Share), delivered as [`OsEvent::Open`].
fn listen_for_opens(java: Java, ctx: egui::Context) -> pdfcraft_ui_egui::OsEventsFn {
    let (tx, rx) = mpsc::channel::<Vec<String>>();
    let started = std::thread::Builder::new().name("open-with".into()).spawn(move || {
        // `waitForOpens` blocks until a file arrives (the launch intent's file comes first).
        while let Some(answer) = java.call(jni_str!("waitForOpens"), "") {
            let paths: Vec<String> = picker_request::decode(&answer).into_iter().map(|p| p.to_string_lossy().into_owned()).collect();
            if paths.is_empty() {
                continue;
            }
            if tx.send(paths).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    if let Err(e) = started {
        log::warn!("couldn't listen for Open with: {e}");
    }
    Box::new(move || rx.try_iter().map(OsEvent::Open).collect())
}

/// Share: the PDF is written to `cache/share/<n>/<name>` (the folder the activity's
/// FileProvider serves) and handed to `PdfCraftActivity.share`, which shows the share sheet.
fn share_sheet(java: Java, cache: PathBuf) -> pdfcraft_ui_egui::ShareFn {
    Box::new(move |name, bytes| {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        let root = cache.join("share");
        forget_old_shares(&root);
        let dir = root.join(stamp.to_string());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join(crate::picker_request::file_name(name));
        std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
        match java.call(jni_str!("share"), &file.to_string_lossy()).as_deref() {
            Some("1") => Ok(()),
            _ => Err("no app on this device can take it".into()),
        }
    })
}

/// Removes copies shared over an hour ago: the app they went to has long since read them.
fn forget_old_shares(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let hour = std::time::Duration::from_secs(3600);
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > hour);
        if old && let Err(e) = std::fs::remove_dir_all(entry.path()) {
            log::warn!("couldn't remove an old shared copy: {e}");
        }
    }
}

/// The system document pickers, through `PdfCraftActivity.pick`.
struct Pickers {
    java: Java,
}

impl pdfcraft_android_rfd::Backend for Pickers {
    fn show(&self, request: pdfcraft_android_rfd::Request, reply: pdfcraft_android_rfd::Reply) {
        use pdfcraft_android_rfd::Kind;
        let kind = match request.kind {
            Kind::PickFile => "file",
            Kind::PickFiles => "files",
            Kind::PickFolder => "folder",
            Kind::SaveFile => "save",
        };
        let extensions: Vec<String> = request.filters.iter().flat_map(|f| f.extensions.iter().cloned()).collect();
        let line = picker_request::encode(kind, request.title.as_deref(), request.file_name.as_deref(), &extensions);
        let java = self.java.clone();
        // `pick` blocks until the user answers, so it never runs on the UI thread. If the thread
        // can't start, `reply` was moved into it and dropped: the picker reads as cancelled.
        let started = std::thread::Builder::new().name("android-picker".into()).spawn(move || {
            let answer = java.call(jni_str!("pick"), &line).unwrap_or_default();
            reply(picker_request::decode(&answer));
        });
        if let Err(e) = started {
            log::warn!("couldn't show the file picker: {e}");
        }
    }
}

/// The system clipboard's text, through `PdfCraftActivity.clipboardGet` / `clipboardSet`.
struct Clipboard {
    java: Java,
}

impl pdfcraft_android_clipboard::TextBackend for Clipboard {
    fn get(&self) -> Option<String> {
        self.java.call(jni_str!("clipboardGet"), "")
    }

    fn set(&self, text: &str) -> bool {
        self.java.call(jni_str!("clipboardSet"), text).as_deref() == Some("1")
    }
}
