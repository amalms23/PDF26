package ai.storyteller.pdfcraft;

import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.FileObserver;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.util.Log;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.webkit.MimeTypeMap;
import android.widget.Toast;

import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * PdfCraft's activity: GameActivity runs the Rust app (libpdfcraft_android.so), and this class
 * adds the Android services the app reaches through JNI (apps/pdfcraft-android/src/java.rs).
 *
 * <p>Every method Rust calls takes one String and returns a String (or null), and blocks the
 * calling (native, non-UI) thread until it has an answer:
 *
 * <ul>
 *   <li>{@link #pick}: the system document pickers. Chosen documents are copied into the app's
 *       cache and their local paths returned, because the app reads and writes files by path.
 *       Writes to those paths are copied back to the documents (see {@link #watch}).
 *   <li>{@link #waitForOpens}: files other apps send ("Open with", Share).
 *   <li>{@link #clipboardGet}, {@link #clipboardSet}: the clipboard's text.
 * </ul>
 */
public class PdfCraftActivity extends GameActivity {
    private static final String TAG = "pdfcraft";
    private static final int REQUEST_BASE = 4200;

    /** Pickers waiting for onActivityResult, by request code. */
    private final ConcurrentHashMap<Integer, Pending> pending = new ConcurrentHashMap<>();
    private final AtomicInteger nextRequest = new AtomicInteger(0);
    /** Local paths of files sent by other apps, waiting for the Rust side. */
    private final LinkedBlockingQueue<String> opens = new LinkedBlockingQueue<>();
    /** Copies run off the UI thread, one at a time, in order. */
    private final ExecutorService io = Executors.newSingleThreadExecutor();
    /** FileObservers stop when collected: keep every one alive. */
    private static final List<FileObserver> observers = Collections.synchronizedList(new ArrayList<>());

    private static final class Pending {
        final String kind;
        final String fileName;
        final CompletableFuture<String> answer = new CompletableFuture<>();

        Pending(String kind, String fileName) {
            this.kind = kind;
            this.fileName = fileName;
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        hideSystemBars();
        // A recreated activity already handled the intent that started it.
        if (savedInstanceState == null) {
            receive(getIntent());
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        }
    }

    /**
     * Full screen: the app draws its own title bar and menus where the status bar would be, so the
     * system bars are hidden and come back for a moment on a swipe from the edge.
     */
    private void hideSystemBars() {
        WindowInsetsControllerCompat bars = WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        bars.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
        bars.hide(WindowInsetsCompat.Type.systemBars());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        receive(intent);
    }

    // ---- Open with / Share ------------------------------------------------------------------

    private void receive(Intent intent) {
        if (intent == null) {
            return;
        }
        Uri uri = null;
        if (Intent.ACTION_VIEW.equals(intent.getAction())) {
            uri = intent.getData();
        } else if (Intent.ACTION_SEND.equals(intent.getAction())) {
            uri = intent.getParcelableExtra(Intent.EXTRA_STREAM);
        }
        if (uri == null) {
            return;
        }
        final Uri source = uri;
        final boolean writable = (intent.getFlags() & Intent.FLAG_GRANT_WRITE_URI_PERMISSION) != 0;
        io.execute(() -> {
            String path = copyIn(source, newFolder("open"), null);
            if (path == null) {
                toast("Couldn't open the file you shared.");
                return;
            }
            if (writable) {
                watch(new File(path), source);
            }
            opens.add(path);
        });
    }

    /** Blocks until another app sends files; returns their local paths, one per line. */
    public String waitForOpens(String unused) {
        try {
            List<String> paths = new ArrayList<>();
            paths.add(opens.take());
            opens.drainTo(paths);
            return String.join("\n", paths);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return null;
        }
    }

    // ---- Pickers ----------------------------------------------------------------------------

    /**
     * Shows a system picker and blocks until it is answered. {@code request} is four lines: kind
     * (file, files, folder, save), title, suggested file name, comma-separated extensions.
     * Returns the chosen local paths, one per line; empty when cancelled.
     */
    public String pick(String request) {
        String[] f = (request + "\n\n\n\n").split("\n", -1);
        String kind = f[0];
        String title = f[1];
        String fileName = f[2];
        String[] extensions = f[3].isEmpty() ? new String[0] : f[3].split(",");

        Intent intent;
        switch (kind) {
            case "folder":
                intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
                break;
            case "save":
                intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType(mimeFor(fileName.isEmpty() && extensions.length > 0 ? "x." + extensions[0] : fileName));
                if (!fileName.isEmpty()) {
                    intent.putExtra(Intent.EXTRA_TITLE, fileName);
                }
                break;
            default:
                intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                setTypes(intent, extensions);
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, "files".equals(kind));
                break;
        }
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);

        int code = REQUEST_BASE + (nextRequest.getAndIncrement() & 0xfff);
        Pending p = new Pending(kind, fileName);
        pending.put(code, p);
        final Intent shown = intent;
        runOnUiThread(() -> {
            try {
                startActivityForResult(shown, code);
            } catch (ActivityNotFoundException e) {
                Log.w(TAG, "no app can answer " + shown.getAction(), e);
                pending.remove(code);
                toast("No app on this device can choose files.");
                p.answer.complete("");
            }
        });
        try {
            return p.answer.get();
        } catch (Exception e) {
            return "";
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        Pending p = pending.remove(requestCode);
        if (p == null) {
            super.onActivityResult(requestCode, resultCode, data);
            return;
        }
        if (resultCode != RESULT_OK || data == null) {
            p.answer.complete("");
            return;
        }
        io.execute(() -> {
            try {
                p.answer.complete(answer(p, data));
            } catch (RuntimeException e) {
                Log.w(TAG, "picker answer failed", e);
                p.answer.complete("");
            }
        });
    }

    private String answer(Pending p, Intent data) {
        ContentResolver resolver = getContentResolver();
        switch (p.kind) {
            case "save": {
                Uri uri = data.getData();
                if (uri == null) {
                    return "";
                }
                String name = sanitize(displayName(uri, p.fileName.isEmpty() ? "document.pdf" : p.fileName));
                File file = new File(newFolder("save"), name);
                watch(file, uri);
                return file.getAbsolutePath();
            }
            case "folder": {
                Uri tree = data.getData();
                if (tree == null) {
                    return "";
                }
                keepPermission(tree);
                File dir = newFolder("folder");
                watchFolder(dir, tree);
                return dir.getAbsolutePath();
            }
            default: {
                List<Uri> uris = new ArrayList<>();
                ClipData clip = data.getClipData();
                if (clip != null) {
                    for (int i = 0; i < clip.getItemCount(); i++) {
                        uris.add(clip.getItemAt(i).getUri());
                    }
                } else if (data.getData() != null) {
                    uris.add(data.getData());
                }
                List<String> paths = new ArrayList<>();
                File dir = newFolder("open");
                for (Uri uri : uris) {
                    boolean writable = keepPermission(uri);
                    String path = copyIn(uri, dir, null);
                    if (path == null) {
                        toast("Couldn't read " + displayName(uri, "a file") + ".");
                        continue;
                    }
                    // Save writes back to the document the user picked.
                    if (writable) {
                        watch(new File(path), uri);
                    }
                    paths.add(path);
                }
                return String.join("\n", paths);
            }
        }
    }

    // ---- Stylus -----------------------------------------------------------------------------

    /** How long after its last event a stylus still counts as near the screen (ms). */
    private static final long NEAR_MS = 600;
    /** The last touch to go down: 0 a finger, 1 a stylus, 2 a stylus erasing (its eraser end, or
     *  its side button held). */
    private volatile int lastDownKind;
    /** A stylus is touching the screen or hovering over it, as of `lastStylusAt`. */
    private volatile boolean stylusNear;
    private volatile long lastStylusAt;
    /** The finger gesture in progress began while the stylus was near: a resting palm. */
    private boolean palm;
    /** Only the stylus marks the page: the app's setting, which `touchTool` keeps up to date. */
    private volatile boolean stylusOnly = true;

    private static boolean isStylus(MotionEvent e, int index) {
        int tool = e.getToolType(index);
        return tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER;
    }

    private static boolean hasStylus(MotionEvent e) {
        for (int i = 0; i < e.getPointerCount(); i++) {
            if (isStylus(e, i)) return true;
        }
        return false;
    }

    private void sawStylus(boolean near) {
        stylusNear = near;
        lastStylusAt = android.os.SystemClock.uptimeMillis();
    }

    private boolean stylusIsNear() {
        return stylusNear && android.os.SystemClock.uptimeMillis() - lastStylusAt < NEAR_MS;
    }

    /** A contact wider than this (millimetres) is the side of a hand, not a fingertip. */
    private static final float PALM_MM = 14f;
    /** A finger gesture is being passed to the app (not dropped as a palm). */
    private boolean fingersPassing;

    /** Whether contact `i` of `e` is as wide as a palm. */
    private boolean palmSized(MotionEvent e, int i) {
        float dpi = getResources().getDisplayMetrics().xdpi;
        return dpi > 0 && e.getTouchMajor(i) / dpi * 25.4f > PALM_MM;
    }

    private boolean anyPalmSized(MotionEvent e) {
        for (int i = 0; i < e.getPointerCount(); i++) {
            if (!isStylus(e, i) && palmSized(e, i)) return true;
        }
        return false;
    }

    private static int firstStylus(MotionEvent e) {
        for (int i = 0; i < e.getPointerCount(); i++) {
            if (isStylus(e, i)) return i;
        }
        return -1;
    }

    /**
     * The pen's part of `e` (pointer `s`), as an event of its own with `action`, its in-between
     * samples kept so the stroke stays as smooth: the hand under the pen is left out.
     */
    private static MotionEvent penOnly(MotionEvent e, int s, int action) {
        MotionEvent.PointerProperties[] props = { new MotionEvent.PointerProperties() };
        e.getPointerProperties(s, props[0]);
        MotionEvent.PointerCoords[] coords = { new MotionEvent.PointerCoords() };
        int history = action == MotionEvent.ACTION_MOVE ? e.getHistorySize() : 0;
        MotionEvent out = null;
        for (int k = 0; k <= history; k++) {
            long time;
            if (k < history) {
                e.getHistoricalPointerCoords(s, k, coords[0]);
                time = e.getHistoricalEventTime(k);
            } else {
                e.getPointerCoords(s, coords[0]);
                time = e.getEventTime();
            }
            if (out == null) {
                out = MotionEvent.obtain(e.getDownTime(), time, action, 1, props, coords, e.getMetaState(), e.getButtonState(),
                        e.getXPrecision(), e.getYPrecision(), e.getDeviceId(), e.getEdgeFlags(), e.getSource(), e.getFlags());
            } else {
                out.addBatch(time, coords, e.getMetaState());
            }
        }
        return out;
    }

    /** Hands the app `ev` and recycles it. */
    private boolean pass(MotionEvent ev) {
        boolean handled = super.dispatchTouchEvent(ev);
        ev.recycle();
        return handled;
    }

    /** Takes back the finger gesture the app has been given (it was a palm): the app undoes its pan. */
    private void cancelFingers(MotionEvent e) {
        MotionEvent cancel = MotionEvent.obtain(e);
        cancel.setAction(MotionEvent.ACTION_CANCEL);
        pass(cancel);
        fingersPassing = false;
    }

    /**
     * Palm rejection, so writing never moves the page:
     * - a hand that comes down while the pen is on or over the screen, or that is as wide as a
     *   palm, is not a touch, from its first event to its last;
     * - a hand that was taken for a finger (it came down before the pen was near) is cancelled
     *   when the pen touches down or the contact spreads to a palm's width, and the app takes
     *   back the pan it made;
     * - while the pen is down, the app gets the pen alone, never the hand beside it (which would
     *   pinch or pan the page).
     */
    @Override
    public boolean dispatchTouchEvent(MotionEvent e) {
        int action = e.getActionMasked();
        int s = firstStylus(e);
        if (s >= 0) {
            sawStylus(action != MotionEvent.ACTION_UP && action != MotionEvent.ACTION_CANCEL);
            boolean down = action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN;
            boolean penMoved = e.getActionIndex() == s;
            if (down && penMoved) {
                boolean button = (e.getButtonState() & MotionEvent.BUTTON_STYLUS_PRIMARY) != 0;
                lastDownKind = (e.getToolType(s) == MotionEvent.TOOL_TYPE_ERASER || button) ? 2 : 1;
            }
            boolean fingers = e.getPointerCount() > 1;
            if (fingersPassing) {
                // The hand already down was a palm after all.
                cancelFingers(e);
                palm = fingers;
                return pass(penOnly(e, s, action == MotionEvent.ACTION_CANCEL ? MotionEvent.ACTION_CANCEL : MotionEvent.ACTION_DOWN));
            }
            if (!fingers) {
                return super.dispatchTouchEvent(e);
            }
            // A hand beside the pen: the app gets the pen alone, and the rest of the hand's
            // gesture once the pen lifts is dropped too.
            palm = true;
            int penAction;
            if (action == MotionEvent.ACTION_CANCEL) {
                penAction = MotionEvent.ACTION_CANCEL;
            } else if (down && penMoved) {
                penAction = MotionEvent.ACTION_DOWN;
            } else if ((action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_POINTER_UP) && penMoved) {
                penAction = MotionEvent.ACTION_UP;
            } else {
                penAction = MotionEvent.ACTION_MOVE;
            }
            return pass(penOnly(e, s, penAction));
        }
        // Fingers alone.
        if (action == MotionEvent.ACTION_DOWN) {
            palm = stylusOnly && (stylusIsNear() || palmSized(e, 0));
            fingersPassing = !palm;
            if (!palm) {
                lastDownKind = 0;
            }
        } else if (fingersPassing && stylusOnly && anyPalmSized(e)) {
            // The contact spread to a palm's width: not a finger after all.
            cancelFingers(e);
            palm = true;
        }
        if (palm) {
            if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
                palm = false;
            }
            return true;
        }
        if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
            fingersPassing = false;
        }
        return super.dispatchTouchEvent(e);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent e) {
        if (hasStylus(e)) {
            int action = e.getActionMasked();
            if (action == MotionEvent.ACTION_HOVER_ENTER || action == MotionEvent.ACTION_HOVER_MOVE) {
                sawStylus(true);
            } else if (action == MotionEvent.ACTION_HOVER_EXIT) {
                sawStylus(false);
            } else if (action == MotionEvent.ACTION_BUTTON_PRESS
                    && (e.getActionButton() & (MotionEvent.BUTTON_STYLUS_PRIMARY | MotionEvent.BUTTON_STYLUS_SECONDARY)) != 0) {
                // A pen button pressed with the pen over the screen, not on it: switch to the
                // eraser and back, as a tap does (pressed on the screen, it erases while held).
                stylusTaps.incrementAndGet();
            }
        }
        return super.dispatchGenericMotionEvent(e);
    }

    /**
     * What the touch that went down last is: "1" a stylus, "2" a stylus erasing (eraser end or
     * side button), "0" a finger. `stylusOnly`
     * is the app's Stylus only setting ("1" on), which palm rejection follows.
     */
    public String touchTool(String stylusOnly) {
        this.stylusOnly = "1".equals(stylusOnly);
        return Integer.toString(lastDownKind);
    }

    // ---- Keyboard ---------------------------------------------------------------------------

    /** Enter (Go, Done…) pressed on the soft keyboard since the app last asked. */
    private final AtomicInteger editorActions = new AtomicInteger();

    /**
     * The soft keyboard's Enter key reaches GameActivity as an editor action, which the Rust side
     * never hears of (android-activity drops it): count it for {@link #takeKeys}.
     */
    @Override
    public void onEditorAction(int action) {
        editorActions.incrementAndGet();
        super.onEditorAction(action);
    }

    /** Stylus button presses (a side button, or the tail or tap of a pen) since the app last asked. */
    private final AtomicInteger stylusTaps = new AtomicInteger();

    /**
     * A pen's button pressed away from the screen can arrive as a stylus key (Android 14 and
     * later, when the system does not keep these keys for its own shortcuts;
     * pressed while the pen touches, it is a button of the touch instead, which `touchTool`
     * reports). Count each press for {@link #takeKeys}; the key goes on, so the app wakes up.
     */
    @Override
    public boolean dispatchKeyEvent(KeyEvent e) {
        switch (e.getKeyCode()) {
            case KeyEvent.KEYCODE_STYLUS_BUTTON_PRIMARY:
            case KeyEvent.KEYCODE_STYLUS_BUTTON_SECONDARY:
            case KeyEvent.KEYCODE_STYLUS_BUTTON_TERTIARY:
            case KeyEvent.KEYCODE_STYLUS_BUTTON_TAIL:
                if (e.getAction() == KeyEvent.ACTION_DOWN && e.getRepeatCount() == 0) {
                    stylusTaps.incrementAndGet();
                }
                break;
            default:
                break;
        }
        return super.dispatchKeyEvent(e);
    }

    /**
     * What the keys did since the last call: "enters taps", how many times the soft keyboard's
     * Enter was pressed and how many times a stylus button was.
     */
    public String takeKeys(String unused) {
        return editorActions.getAndSet(0) + " " + stylusTaps.getAndSet(0);
    }

    // ---- Clipboard --------------------------------------------------------------------------

    public String clipboardGet(String unused) {
        ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm == null || !cm.hasPrimaryClip()) {
            return null;
        }
        ClipData clip = cm.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) {
            return null;
        }
        CharSequence text = clip.getItemAt(0).coerceToText(this);
        return text == null ? null : text.toString();
    }

    public String clipboardSet(String text) {
        ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm == null) {
            return "0";
        }
        cm.setPrimaryClip(ClipData.newPlainText("PdfCraft", text));
        return "1";
    }

    // ---- Files ------------------------------------------------------------------------------

    /** A new, empty folder in the cache for one picker's files, so names never collide. */
    private File newFolder(String purpose) {
        File dir = new File(getCacheDir(), "documents/" + purpose + "/" + System.nanoTime());
        if (!dir.mkdirs() && !dir.isDirectory()) {
            Log.w(TAG, "couldn't create " + dir);
        }
        return dir;
    }

    /** Copies a document into {@code dir}; returns its local path, or null. */
    private String copyIn(Uri uri, File dir, String name) {
        String fileName = sanitize(name != null ? name : displayName(uri, "document.pdf"));
        File out = new File(dir, fileName);
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream os = new FileOutputStream(out)) {
            if (in == null) {
                return null;
            }
            copy(in, os);
            return out.getAbsolutePath();
        } catch (IOException | SecurityException e) {
            Log.w(TAG, "couldn't read " + uri, e);
            return null;
        }
    }

    /**
     * Copies {@code file} to {@code target} every time the app finishes writing it. The app saves
     * by writing a hidden temporary file in the same folder and renaming it over the file
     * (MOVED_TO); other writers close the file itself (CLOSE_WRITE).
     */
    private void watch(File file, Uri target) {
        final String name = file.getName();
        final File dir = file.getParentFile();
        if (dir == null) {
            return;
        }
        FileObserver observer = new FileObserver(dir.getAbsolutePath(), FileObserver.CLOSE_WRITE | FileObserver.MOVED_TO) {
            @Override
            public void onEvent(int event, String path) {
                if (name.equals(path)) {
                    io.execute(() -> copyOut(file, target));
                }
            }
        };
        observer.startWatching();
        observers.add(observer);
    }

    /** Copies each file the app writes into {@code dir} to a new document in the chosen folder. */
    private void watchFolder(File dir, Uri tree) {
        FileObserver observer = new FileObserver(dir.getAbsolutePath(), FileObserver.CLOSE_WRITE | FileObserver.MOVED_TO) {
            @Override
            public void onEvent(int event, String path) {
                if (path == null || path.startsWith(".")) {
                    return;
                }
                File file = new File(dir, path);
                io.execute(() -> {
                    try {
                        Uri parent = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree));
                        Uri doc = DocumentsContract.createDocument(getContentResolver(), parent, mimeFor(path), path);
                        if (doc == null) {
                            toast("Couldn't save " + path + " to that folder.");
                            return;
                        }
                        copyOut(file, doc);
                    } catch (IOException | RuntimeException e) {
                        Log.w(TAG, "couldn't save " + path + " to the folder", e);
                        toast("Couldn't save " + path + " to that folder.");
                    }
                });
            }
        };
        observer.startWatching();
        observers.add(observer);
    }

    private void copyOut(File file, Uri target) {
        try (InputStream in = new FileInputStream(file);
             OutputStream os = getContentResolver().openOutputStream(target, "wt")) {
            if (os == null) {
                throw new IOException("no output stream");
            }
            copy(in, os);
            Log.i(TAG, "saved " + file.getName() + " to " + target);
        } catch (IOException | RuntimeException e) {
            Log.w(TAG, "couldn't save to " + target, e);
            toast("Couldn't save " + file.getName() + ". Use Save As to pick another place.");
        }
    }

    private static void copy(InputStream in, OutputStream out) throws IOException {
        byte[] buf = new byte[1 << 16];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
    }

    /** Keeps access to a picked document across restarts; true when it may be written. */
    private boolean keepPermission(Uri uri) {
        ContentResolver resolver = getContentResolver();
        try {
            resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            return true;
        } catch (SecurityException e) {
            try {
                resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (SecurityException ignored) {
                // Readable for now only; the copy is made right away.
            }
            return false;
        }
    }

    private String displayName(Uri uri, String fallback) {
        try (Cursor c = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) {
                return c.getString(0);
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "no name for " + uri, e);
        }
        String last = uri.getLastPathSegment();
        return last != null && !last.isEmpty() ? last : fallback;
    }

    /** A name that stays one visible file in its folder. */
    private static String sanitize(String name) {
        String clean = name.replace('/', '_').replace('\\', '_').replace('\0', '_').trim();
        while (clean.startsWith(".")) {
            clean = clean.substring(1);
        }
        return clean.isEmpty() ? "document.pdf" : clean;
    }

    private static String mimeFor(String fileName) {
        int dot = fileName.lastIndexOf('.');
        String ext = dot >= 0 ? fileName.substring(dot + 1).toLowerCase(Locale.ROOT) : "";
        String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
        if (mime == null && ext.equals("pdf")) {
            mime = "application/pdf";
        }
        return mime != null ? mime : "application/octet-stream";
    }

    private static void setTypes(Intent intent, String[] extensions) {
        List<String> types = new ArrayList<>();
        for (String ext : extensions) {
            String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext.toLowerCase(Locale.ROOT));
            if (mime == null && ext.equalsIgnoreCase("pdf")) {
                mime = "application/pdf";
            }
            if (mime == null) {
                // An extension Android doesn't know (a .p12, say): let the user choose any file.
                types.clear();
                break;
            }
            if (!types.contains(mime)) {
                types.add(mime);
            }
        }
        if (types.size() == 1) {
            intent.setType(types.get(0));
        } else {
            intent.setType("*/*");
            if (!types.isEmpty()) {
                intent.putExtra(Intent.EXTRA_MIME_TYPES, types.toArray(new String[0]));
            }
        }
    }

    private void toast(String message) {
        runOnUiThread(() -> Toast.makeText(this, message, Toast.LENGTH_LONG).show());
    }
}
