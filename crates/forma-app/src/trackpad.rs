//! GPUI 0.2 exposes precise scroll, but not AppKit magnification events.
//! A window-scoped local monitor forwards pinch deltas to the foreground task.
use crate::app::Studio;
use block2::RcBlock;
use gpui::{Context, Window, point, px};
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSPoint, NSRect};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(crate) struct PinchMonitor(Retained<AnyObject>);

impl PinchMonitor {
    pub(crate) fn install(window: &Window, cx: &mut Context<Studio>) -> Option<Self> {
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        // SAFETY: GPUI lends the live view on AppKit's main thread. Only the window
        // number is captured, never a borrowed native pointer.
        let number: isize = unsafe {
            let view = handle.ns_view.as_ptr().cast::<AnyObject>();
            let native: *mut AnyObject = msg_send![view, window];
            msg_send![native, windowNumber]
        };
        let (sender, receiver) = async_channel::unbounded();
        let block = RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
            // SAFETY: AppKit invokes local monitors on the main thread with a live
            // NSEvent. The magnify mask guarantees that magnification is available.
            unsafe {
                let event_number: isize = msg_send![event, windowNumber];
                if event_number == number {
                    let magnification: f64 = msg_send![event, magnification];
                    let location: NSPoint = msg_send![event, locationInWindow];
                    let native: *mut AnyObject = msg_send![event, window];
                    let view: *mut AnyObject = msg_send![native, contentView];
                    let bounds: NSRect = msg_send![view, bounds];
                    let _ = sender.try_send((
                        location.x,
                        bounds.size.height - location.y,
                        magnification,
                    ));
                }
            }
            event
        });
        // SAFETY: AppKit copies the block; retain the autoreleased monitor token
        // until Drop removes the callback. No global event tap or permission needed.
        let monitor = unsafe {
            let token: *mut AnyObject = msg_send![class!(NSEvent),
                addLocalMonitorForEventsMatchingMask: (1_usize << 30),
                handler: &*block
            ];
            Retained::retain(token)?
        };
        cx.spawn(async move |this, cx| {
            while let Ok((x, y, magnification)) = receiver.recv().await {
                if this
                    .update(cx, |studio, cx| {
                        let position = point(px(x as f32), px(y as f32));
                        studio.magnify(position, magnification as f32, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Some(Self(monitor))
    }
}

impl Drop for PinchMonitor {
    fn drop(&mut self) {
        // SAFETY: Studio and its monitor are created/dropped on the main thread.
        unsafe {
            let _: () = msg_send![class!(NSEvent), removeMonitor: &*self.0];
        }
    }
}
