//! macOS hands `rift://` links (`CFBundleURLTypes` in `Rift.app`) to the
//! app as a `kAEGetURL` Apple Event, and folders (`CFBundleDocumentTypes`:
//! dropped on the Dock icon, opened with Rift from Finder, `open -a Rift
//! <dir>`, an editor's "external terminal") as an open-documents event,
//! which AppKit passes to the application delegate's
//! `application:openURLs:`. Both are hooked before the event loop runs, so
//! what launched Rift arrives too: it waits until the first window is up
//! ([`take_launch_links`]: the first tab opens in that folder), later ones
//! become `UserEvent::OpenLink` (`app/links.rs` opens them).

use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, NSObject, Sel};
use objc2::{AnyThread, define_class, msg_send, sel};
use objc2_foundation::NSString;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// `'GURL'`: both the event class (`kInternetEventClass`) and its id
/// (`kAEGetURL`).
const GURL: u32 = u32::from_be_bytes(*b"GURL");
/// `'----'` (`keyDirectObject`): the link itself.
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();
/// Links that arrived before the first window was up; `None` once the app
/// took them.
static LAUNCH: Mutex<Option<Vec<String>>> = Mutex::new(Some(Vec::new()));

/// Hand `link` to the app: kept for the first window while it is not up
/// yet, else as a user event.
fn deliver(link: String) {
    if let Ok(mut launch) = LAUNCH.lock()
        && let Some(waiting) = launch.as_mut()
    {
        waiting.push(link);
        return;
    }
    if let Some(proxy) = PROXY.get().and_then(|p| p.lock().ok()) {
        let _ = proxy.send_event(UserEvent::OpenLink(link));
    }
}

/// The links that arrived before the first window was up (the one that
/// launched Rift); later ones arrive as user events.
pub fn take_launch_links() -> Vec<String> {
    LAUNCH
        .lock()
        .ok()
        .and_then(|mut l| l.take())
        .unwrap_or_default()
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "RiftLinkHandler"]
    struct Handler;

    impl Handler {
        #[unsafe(method(handleGetURL:withReplyEvent:))]
        fn handle(&self, event: &AnyObject, _reply: &AnyObject) {
            // SAFETY: `event` is the NSAppleEventDescriptor AppKit passes;
            // both messages exist on it and return an object or nil
            let link: Option<Retained<NSString>> = unsafe {
                let param: Option<Retained<AnyObject>> =
                    msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
                match param {
                    Some(p) => msg_send![&*p, stringValue],
                    None => None,
                }
            };
            let Some(link) = link else { return };
            deliver(link.to_string());
        }
    }
);

/// `- (void)application:(NSApplication *)app openURLs:(NSArray<NSURL *> *)urls`,
/// added to winit's application delegate: folders and files (an
/// open-documents event) and, once AppKit routes them here, `rift://` links.
extern "C-unwind" fn open_urls(
    _this: *mut AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
    urls: *mut AnyObject,
) {
    if urls.is_null() {
        return;
    }
    // SAFETY: `urls` is the NSArray of NSURL AppKit passes; `count`,
    // `objectAtIndex:` and `absoluteString` exist on them
    let strings: Vec<String> = unsafe {
        let count: usize = msg_send![urls, count];
        (0..count)
            .filter_map(|i| {
                let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                if url.is_null() {
                    return None;
                }
                let s: Option<Retained<NSString>> = msg_send![url, absoluteString];
                s.map(|s| s.to_string())
            })
            .collect()
    };
    for url in strings {
        match rift_core::uri::opened_url_link(&url) {
            Ok(link) => deliver(link),
            Err(e) => log::warn!("open: {e}"),
        }
    }
}

/// Route `rift://` links and opened folders to `proxy` for as long as the
/// process runs.
pub fn install(proxy: EventLoopProxy<UserEvent>) {
    if PROXY.set(Mutex::new(proxy)).is_err() {
        return;
    }
    // SAFETY: a plain NSObject subclass with no ivars
    let handler: Retained<Handler> = unsafe { msg_send![Handler::alloc(), init] };
    // SAFETY: the shared manager exists in every AppKit process; the
    // selector is the handler's own method with the signature AppKit calls
    unsafe {
        let manager: Retained<AnyObject> =
            msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager];
        let _: () = msg_send![
            &*manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleGetURL:withReplyEvent:),
            forEventClass: GURL,
            andEventID: GURL
        ];
    }
    // the manager does not retain its handlers: it lives with the process
    std::mem::forget(handler);
    add_open_urls();
}

/// Give winit's application delegate `application:openURLs:`. AppKit's
/// own open-documents handler (installed while the app finishes launching,
/// over any installed earlier) calls it; without it AppKit would try to
/// open the folder as a document and show an error.
fn add_open_urls() {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    // SAFETY: `delegate` is winit's application delegate (set when the
    // event loop was built), alive for the whole run; the method has the
    // selector's exact signature (type "v@:@@"); `class_addMethod` leaves
    // an existing implementation alone
    unsafe {
        let delegate: *mut AnyObject = msg_send![&*app, delegate];
        if delegate.is_null() {
            log::warn!("open: no application delegate; folders opened with Rift are ignored");
            return;
        }
        let cls = objc2::ffi::object_getClass(delegate) as *mut AnyClass;
        let imp: Imp = std::mem::transmute::<
            extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            Imp,
        >(open_urls);
        let added =
            objc2::ffi::class_addMethod(cls, sel!(application:openURLs:), imp, c"v@:@@".as_ptr());
        if !added.as_bool() {
            log::warn!("open: the application delegate already handles opened URLs");
        }
    }
}
