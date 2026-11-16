//! PdfCraft for Android: the same egui app as the desktop, started by a GameActivity.
//!
//! The Gradle project in `android/` packages this library (`libpdfcraft_android.so`) with
//! `PdfCraftActivity`, a GameActivity that adds what egui can't reach on its own: the system
//! document pickers, "Open with" from other apps, and the clipboard. This crate connects those to
//! the app through the stand-ins `pdfcraft-ui-egui` uses for rfd and arboard on Android.
//!
//! Build steps: `apps/pdfcraft-android/README.md`. On other platforms this crate is empty.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod java;
#[cfg(target_os = "android")]
mod keyboard;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod ime_diff;
mod picker_request;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod taps;
