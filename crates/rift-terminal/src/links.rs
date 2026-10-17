//! macOS hands `rift://` links (`CFBundleURLTypes` in `Rift.app`) to the
//! app as a `kAEGetURL` Apple Event. The handler is installed before the
//! event loop runs, so the link that launched Rift arrives too; it becomes
//! `UserEvent::OpenLink` (`app/links.rs` opens it).

use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
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
            if let Some(proxy) = PROXY.get().and_then(|p| p.lock().ok()) {
                let _ = proxy.send_event(UserEvent::OpenLink(link.to_string()));
            }
        }
    }
);

/// Route `rift://` links to `proxy` for as long as the process runs.
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
}
