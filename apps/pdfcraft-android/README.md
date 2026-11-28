# PDF26: PdfCraft for Android

The full PdfCraft app (`pdfcraft-ui-egui`, the same interface as on the desktop) running natively
on Android phones and tablets, through egui's Android support and a GameActivity.

## How it fits together

- `src/` (Rust, `libpdfcraft_android.so`): `android_main` starts the egui app, and connects the
  Android services below to it.
- `android/` (Gradle): the APK. `PdfCraftActivity` is a GameActivity that adds what egui can't
  reach on its own:
  - **Open / Save / folders** use the system document pickers. Picked documents are copied into
    the app's cache and opened by path, as on the desktop. Saving a picked document writes it back
    to where it came from; Save As writes the new document the user chose.
  - **Open with** and **Share** from other apps (PDFs) open in a new tab.
  - **Clipboard** text goes to and from the system clipboard. Images and file lists are not
    copied yet.
- **Typing**: the soft keyboard edits GameActivity's text buffer, which winit ignores.
  `src/keyboard.rs` reads that buffer every frame while a field has focus and passes the changes
  to egui as input-method text (`src/ime_diff.rs`). The keyboard's Enter (Go, Done) reaches
  `PdfCraftActivity.onEditorAction`, which the Rust side collects (`takeEnters`) and sends as the
  Enter key, so the page number box and search boxes act on it.
- **Scrolling**: a finger dragged across the page pans it even with the Select tool on, however
  long it rested first, and the page glides on after a flick, further than a touchpad swipe
  (`touch_pan` in `crates/ui-egui/src/canvas.rs`, `TOUCH_TAU` in `canvas/fling.rs`). A finger
  makes no long press; the stylus selects text, and taps select a word, then a line, never the
  whole page. A tap still opens links and form fields.
- **Stylus**: `PdfCraftActivity` tells the app whether each touch is a stylus or a finger
  (`touchTool`). Once a stylus has touched the screen, only it writes, highlights and draws while
  **Stylus only** is on (in the toolbar's ⋯ menu): a finger pans with every tool. Palm
  rejection: a hand that comes down while the stylus is on or over the screen, or that is as wide
  as a palm, is ignored; one taken for a finger (down before the pen came near) is cancelled when
  the pen touches, and the page goes back to where it was (`touch_pan`); while the pen is down
  the app gets the pen alone, never the hand beside it.
- **Notes toolbar** (`crates/ui-egui/src/notes_bar.rs`), across the top: Undo, Redo, Select,
  Hand, Pen, Highlighter, Eraser, Text, Lasso, the current tool's colours and three sizes, Add
  page (blank, lined, grid or dotted, after the page in view), Share and a ⋯ menu (Stylus only, shapes,
  sticky note). Pen and highlighter strokes are smoothed when they finish.
- **Share**: hands the open PDF, with its unsaved changes, to Android's share sheet (mail, Drive,
  Messages, Print…). The document itself stays unsaved; the copy is written under `cache/share/`,
  served to the other app by a FileProvider, and cleared an hour later.
- **Lasso**: a loop drawn around writing selects it; a drag inside the dashed box moves it all,
  and the toolbar recolours, copies, pastes or deletes it.
- **Hold to snap**: a pen stroke held still for half a second at its end becomes a clean line,
  rectangle, oval, triangle or polygon (`crates/ui-egui/src/tools/comments/snap.rs`); a
  highlighter stroke becomes straight.
- **Highlighter**: freehand, straight (held level along a line of text) or over text (drag across
  the words: a text highlight, with no note opening). One colour for all three.
- **Pen**: the stroke is steadied as it is drawn (slow, shaky moves eased; quick ones followed at
  once), keeps its sharp corners, and is saved with only the points it needs.
- **Gestures**: two fingers tapped together undo, three redo (`src/taps.rs`). The stylus side
  button (or its eraser end) erases while held on the screen; pressed with the pen over the
  screen (or as a stylus key, where the system passes those on), it switches to the eraser and
  back.
- **Full screen**: a button in the top corner leaves it (there is no Esc key to press).
- **Tablet layout**: no Print button (not done yet), keyboard-shortcut help, Discord button or
  shortcut hints; the All tools list starts closed.
- **Name**: the Android build is called PDF26 (`app_name`), with its own launcher icon
  (`android/app/src/main/res/drawable/ic_launcher_*.xml`). As a modified version it shows no
  ArtCraft marks. The home screen offers New notebook (a lined page) and Open a PDF.
- The app runs full screen; swipe from an edge to show the system bars.
- `crates/android-rfd` and `crates/android-clipboard`: stand-ins for rfd and arboard, which have
  no Android backend. On Android `pdfcraft-ui-egui` names them `rfd` and `arboard`
  (`crates/ui-egui/src/lib.rs`), so the app's code is unchanged.

## Build

You need:

- Rust with the Android targets: `rustup target add aarch64-linux-android x86_64-linux-android`
- `cargo install cargo-ndk`
- JDK 17, and the Android SDK with platform 35, build-tools 35 and NDK 27.3.13750724
  (`sdkmanager "platforms;android-35" "build-tools;35.0.0" "ndk;27.3.13750724"`).
  Set `ANDROID_HOME` (or write `sdk.dir` in `android/local.properties`) and `JAVA_HOME`.

From the repository root:

```bash
cargo ndk -t arm64-v8a -t x86_64 -P 26 -o apps/pdfcraft-android/android/app/src/main/jniLibs build --release -p pdfcraft-android
```

cargo-ndk also copies the `cdylib` side-builds of some dependencies (`libboa_engine-*.so`,
`libocrs-*.so`, `librten-*.so`). The app links them statically, so delete them before packaging:

```bash
rm -f apps/pdfcraft-android/android/app/src/main/jniLibs/*/lib{boa_engine,ocrs,rten}-*.so
```

```bash
cd apps/pdfcraft-android/android && ./gradlew assembleDebug
```

The APK is `android/app/build/outputs/apk/debug/app-debug.apk`. Install it with
`adb install -r app-debug.apk`, or copy it to a device and open it (allow installs from that
source when Android asks). Logs: `adb logcat -s pdfcraft`.

`arm64-v8a` is for phones and tablets; `x86_64` is for the emulator. Drop one to build faster.

A release APK (`./gradlew assembleRelease`) is unsigned: sign it with your own key
(`apksigner`) before installing or publishing.

## Not done yet

- Copying images and file lists to the clipboard, and pasting images.
- Printing and digital IDs from the system key store.
- A touch-first layout: the interface is the desktop one, best on a tablet or in landscape.
