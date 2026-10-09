//! macOS: AppKit and the UserNotifications framework.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use block2::{DynBlock, RcBlock};
use futures::channel::oneshot;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation,
    NSDeviceRGBColorSpace, NSGraphicsContext, NSModalResponse, NSModalResponseAbort,
    NSModalResponseOK, NSOpenPanel, NSWorkspace,
};
use objc2_foundation::{
    NSBundle, NSDictionary, NSError, NSLocale, NSPoint, NSRect, NSSize, NSString, NSURL,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use super::FolderPick;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSBeep();
}

pub fn beep() {
    // SAFETY: NSBeep takes no arguments and may be called from any thread.
    unsafe { NSBeep() }
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerRegisterFontsForURL(
        url: &NSURL,
        scope: u32,
        error: *mut *mut std::ffi::c_void,
    ) -> bool;
}

/// `kCTFontManagerScopeProcess`: visible to this process only.
const FONT_SCOPE_PROCESS: u32 = 1;

/// Register a font file with CoreText for this process, so font
/// descriptors (and with them fallback lists) find it by family name.
pub fn register_font_file(path: &Path) {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    // SAFETY: NSURL is toll-free bridged to CFURL, and a null error
    // pointer is allowed. Registering an already registered file fails
    // harmlessly.
    unsafe {
        CTFontManagerRegisterFontsForURL(&url, FONT_SCOPE_PROCESS, std::ptr::null_mut());
    }
}

/// Receives the identifier of a clicked notification.
type ClickHandler = Box<dyn Fn(String)>;

thread_local! {
    static ON_CLICK: RefCell<Option<ClickHandler>> = RefCell::new(None);
    /// The center only holds its delegate weakly.
    static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ChdaNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Show banners while chda is the active app too.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            ON_CLICK.with(|f| {
                if let Some(f) = f.borrow().as_ref() {
                    f(id);
                }
            });
            handler.call(());
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

/// The notification center needs an app bundle; a binary started from
/// `cargo run` has none and would raise an exception.
fn bundled() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

pub fn init_notifications(on_click: impl Fn(String) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if !bundled() {
        return;
    }
    ON_CLICK.with(|f| *f.borrow_mut() = Some(Box::new(on_click)));
    let delegate = Delegate::new(mtm);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
    let done = RcBlock::new(|_: Bool, _: *mut NSError| {});
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
}

pub fn notify(title: &str, body: &str, id: &str) {
    if !bundled() {
        // Unbundled development builds: no click handling.
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            esc(body),
            esc(title)
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        return;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(id),
        &content,
        None,
    );
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
}

/// The user's locale, e.g. `ko_KR`.
pub fn locale_identifier() -> String {
    NSLocale::currentLocale().localeIdentifier().to_string()
}

pub fn show_about() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanel(None);
}

pub fn restore_windows() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    for window in NSApplication::sharedApplication(mtm).windows() {
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
    }
}

/// A new open panel, or `None` when AppKit returns none, as it does when
/// its open panel service fails.
pub fn open_panel() -> Option<Retained<NSOpenPanel>> {
    MainThreadMarker::new()?;
    // SAFETY: `+[NSOpenPanel openPanel]` takes no arguments and runs on the
    // main thread (checked above). It can return nil, which the generated
    // binding declares impossible and objc2 turns into a panic; receiving
    // an optional turns nil into `None`.
    unsafe { msg_send![NSOpenPanel::class(), openPanel] }
}

/// Show `panel` (from [`open_panel`]) for choosing folders, with GPUI's
/// options, and report the outcome on `done`.
pub fn begin_folder_panel(
    panel: Option<Retained<NSOpenPanel>>,
    prompt: &str,
    done: oneshot::Sender<FolderPick>,
) {
    let Some(panel) = panel else {
        let _ = done.send(FolderPick::Unavailable(
            "AppKit could not create an open panel".into(),
        ));
        return;
    };
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(true);
    panel.setCanCreateDirectories(true);
    panel.setResolvesAliases(false);
    panel.setPrompt(Some(&NSString::from_str(prompt)));
    let done = Cell::new(Some(done));
    let handler = RcBlock::new({
        let panel = panel.clone();
        move |response: NSModalResponse| {
            let Some(done) = done.take() else {
                return;
            };
            let _ = done.send(panel_outcome(response, || {
                panel
                    .URLs()
                    .iter()
                    .filter(|url| url.isFileURL())
                    .filter_map(|url| url.to_file_path())
                    .collect()
            }));
        }
    });
    panel.beginWithCompletionHandler(&handler);
}

/// What a closed panel's response means. Apple documents `Abort` as the
/// panel failing to display.
fn panel_outcome(response: NSModalResponse, chosen: impl FnOnce() -> Vec<PathBuf>) -> FolderPick {
    if response == NSModalResponseOK {
        FolderPick::Chosen(chosen())
    } else if response == NSModalResponseAbort {
        FolderPick::Unavailable("the open panel failed to display".into())
    } else {
        FolderPick::Cancelled
    }
}

pub fn set_badge(count: usize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}

/// Where the app with `bundle_id` is installed, if it is.
pub fn app_path(bundle_id: &str) -> Option<String> {
    let url = NSWorkspace::sharedWorkspace()
        .URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?;
    url.path().map(|p| p.to_string())
}

/// The icon of the app at `path` as a PNG, `size` pixels square.
pub fn app_icon_png(path: &str, size: isize) -> Option<Vec<u8>> {
    let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
    // SAFETY: null planes make the rep allocate its own pixel buffer; the
    // other arguments describe 8-bit RGBA.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            size,
            size,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let side = size as f64;
    icon.drawInRect_fromRect_operation_fraction(
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side)),
        NSRect::ZERO,
        NSCompositingOperation::SourceOver,
        1.0,
    );
    NSGraphicsContext::restoreGraphicsState_class();
    // SAFETY: an empty dictionary is a valid set of PNG properties.
    let data = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_open_panel_reports_unavailable() {
        let (done, mut picked) = oneshot::channel();
        begin_folder_panel(None, "Add repository", done);
        assert!(matches!(
            picked.try_recv(),
            Ok(Some(FolderPick::Unavailable(_)))
        ));
    }

    #[test]
    fn panel_responses_map_to_folder_picks() {
        let chosen = || vec![PathBuf::from("/src/a")];
        assert_eq!(
            panel_outcome(NSModalResponseOK, chosen),
            FolderPick::Chosen(chosen())
        );
        assert_eq!(
            panel_outcome(objc2_app_kit::NSModalResponseCancel, chosen),
            FolderPick::Cancelled
        );
        assert!(matches!(
            panel_outcome(NSModalResponseAbort, chosen),
            FolderPick::Unavailable(_)
        ));
    }
}
