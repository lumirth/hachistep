#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod native {
    use gpui_kit::{App, AsyncApp};
    use objc::{
        class, msg_send,
        runtime::{Class, Object, Sel, BOOL, NO, YES},
        sel, sel_impl,
    };
    use std::{
        cell::{Cell, RefCell},
        ffi::{c_char, c_void},
    };
    thread_local! {
        static CONTEXT: RefCell<Option<AsyncApp>> = const { RefCell::new(None) };
        static QUITTING: Cell<bool> = const { Cell::new(false) };
    }
    unsafe extern "C" {
        fn class_addMethod(
            class: *const Class,
            selector: Sel,
            implementation: *const c_void,
            types: *const c_char,
        ) -> BOOL;
    }
    extern "C" fn should_terminate(_: &Object, _: Sel, _: *mut Object) -> usize {
        CONTEXT.with(|context| {
            context.borrow().as_ref().map_or(1, |context| {
                context.update(|cx| {
                    if cx.global::<crate::Desktop>().windows.is_empty() {
                        return 1;
                    }
                    QUITTING.set(true);
                    crate::quit(cx);
                    2
                })
            })
        })
    }
    pub fn install(cx: &App) {
        CONTEXT.with(|context| *context.borrow_mut() = Some(cx.to_async()));
        // GPUI 0.3.9 exposes termination notification, but no cancellable quit request.
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let delegate: *mut Object = msg_send![app, delegate];
            class_addMethod(
                (*delegate).class(),
                sel!(applicationShouldTerminate:),
                should_terminate as *const c_void,
                c"Q@:@".as_ptr(),
            );
        }
    }
    pub fn finish_quit(allow: bool) -> bool {
        if !QUITTING.replace(false) {
            return false;
        }
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () =
                msg_send![app, replyToApplicationShouldTerminate: if allow { YES } else { NO }];
        }
        true
    }
    pub fn bring_all() {
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, arrangeInFront: std::ptr::null::<Object>()];
        }
    }
    pub fn restore_minimized() -> bool {
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let windows: *mut Object = msg_send![app, windows];
            let count: usize = msg_send![windows, count];
            for ix in 0..count {
                let window: *mut Object = msg_send![windows, objectAtIndex: ix];
                let minimized: BOOL = msg_send![window, isMiniaturized];
                if minimized == YES {
                    let _: () = msg_send![window, deminiaturize: std::ptr::null::<Object>()];
                    let _: () = msg_send![window, makeKeyAndOrderFront: std::ptr::null::<Object>()];
                    return true;
                }
            }
        }
        false
    }
}
#[cfg(target_os = "macos")]
pub(crate) use native::*;
#[cfg(not(target_os = "macos"))]
pub(crate) fn install(_: &gpui_kit::App) {}
#[cfg(not(target_os = "macos"))]
pub(crate) fn finish_quit(_: bool) -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn bring_all() {}
#[cfg(not(target_os = "macos"))]
pub(crate) fn restore_minimized() -> bool {
    false
}
