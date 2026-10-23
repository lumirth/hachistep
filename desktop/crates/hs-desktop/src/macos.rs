#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod native {
    use cocoa_foundation::foundation::NSRect;
    use gpui_kit::{App, AsyncApp, Pixels, Task, Window};
    use objc::{
        class, msg_send,
        rc::StrongPtr,
        runtime::{Class, Object, Sel, BOOL, NO, YES},
        sel, sel_impl,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
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
    pub fn resize_settings(window: &mut Window, height: Pixels, cx: &App) -> Option<Task<()>> {
        if window.is_fullscreen() {
            return None;
        }
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return None;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        let view = unsafe { StrongPtr::retain(handle.ns_view.as_ptr().cast::<Object>()) };
        let native: *mut Object = unsafe { msg_send![*view, window] };
        if native.is_null() {
            return None;
        }
        let native = unsafe { StrongPtr::retain(native) };
        Some(cx.foreground_executor().spawn(async move {
            unsafe {
                let current: *mut Object = msg_send![*view, window];
                let visible: BOOL = msg_send![*native, isVisible];
                if current != *native || visible == NO {
                    return;
                }
                let frame: NSRect = msg_send![*native, frame];
                let content: NSRect = msg_send![*view, frame];
                let top = frame.origin.y + frame.size.height;
                let mut target = frame;
                target.size.height += f64::from(f32::from(height)) - content.size.height;
                let screen: *mut Object = msg_send![*native, screen];
                if !screen.is_null() {
                    let available: NSRect = msg_send![screen, visibleFrame];
                    target.size.height = target.size.height.min(available.size.height);
                    target.origin.y = (top.min(available.origin.y + available.size.height) - target.size.height)
                        .max(available.origin.y);
                } else {
                    target.origin.y = top - target.size.height;
                }
                if (frame.size.height - target.size.height).abs() < 1.
                    && (frame.origin.y - target.origin.y).abs() < 1.
                {
                    return;
                }
                let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
                let reduce_motion: BOOL = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
                // Run outside GPUI's update borrow so AppKit resize callbacks can update layout.
                let _: () = msg_send![*native, setFrame: target display: YES animate: if reduce_motion == YES { NO } else { YES }];
            }
        }))
    }
    pub fn bring_all() {
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, arrangeInFront: std::ptr::null::<Object>()];
        }
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
pub(crate) fn resize_settings(
    window: &mut gpui_kit::Window,
    height: gpui_kit::Pixels,
    _: &gpui_kit::App,
) -> Option<gpui_kit::Task<()>> {
    if !window.is_fullscreen() {
        window.resize(gpui_kit::size(window.viewport_size().width, height));
    }
    None
}
