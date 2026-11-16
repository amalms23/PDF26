//! Calls into `PdfCraftActivity` (android/app/src/main/java/.../PdfCraftActivity.java).
//!
//! Every call is an instance method taking one `String` and returning a `String` (or null), so
//! the bridge stays one small function. Calls are made on the activity object itself, which avoids
//! looking up the app's classes from native threads (their class loader can't see them).

use jni::objects::{JObject, JString};
use jni::strings::JNIStr;
use jni::{JValue, JavaVM, jni_sig};

/// The Java VM and the activity, for calls from any thread.
#[derive(Clone)]
pub(crate) struct Java {
    vm: JavaVM,
    /// The activity's JNI global reference, owned by android-activity for the life of the process.
    activity: usize,
}

impl Java {
    /// The VM and activity of the running app.
    pub(crate) fn of(app: &winit::platform::android::activity::AndroidApp) -> Option<Self> {
        let vm = app.vm_as_ptr();
        let activity = app.activity_as_ptr();
        if vm.is_null() || activity.is_null() {
            return None;
        }
        // SAFETY: android-activity hands out the process's JavaVM pointer, valid (and non-null,
        // checked above) for the life of the process.
        #[allow(unsafe_code)]
        let vm = unsafe { JavaVM::from_raw(vm.cast()) };
        Some(Self { vm, activity: activity as usize })
    }

    /// Call `name(String): String` on the activity. `None` for a null answer or any JNI error
    /// (logged; a Java exception is caught and cleared by the attach guard).
    pub(crate) fn call(&self, name: &'static JNIStr, arg: &str) -> Option<String> {
        let result: Result<Option<String>, jni::errors::Error> = self.vm.attach_current_thread(|env| {
            // SAFETY: `activity` is a global reference that outlives this call, and nothing else
            // wraps it on this thread; global references are valid on every thread.
            #[allow(unsafe_code)]
            let activity = unsafe { JObject::from_raw(env, self.activity as jni::sys::jobject) };
            let arg = env.new_string(arg)?;
            let value = env.call_method(&activity, name, jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"), &[JValue::Object(&arg)])?;
            let obj = value.l()?;
            if obj.is_null() {
                return Ok(None);
            }
            let text = env.cast_local::<JString>(obj)?;
            Ok(Some(text.try_to_string(env)?))
        });
        match result {
            Ok(answer) => answer,
            Err(e) => {
                log::warn!("PdfCraftActivity.{name} failed: {e}");
                None
            }
        }
    }
}
