//! 无边框小窗的 AppKit 外观调整。

/// 把无边框窗口裁成圆角：窗口背景透明，内容视图的图层按圆角裁切（网页视图是它的子视图，
/// 一起被裁掉），再让系统按新的轮廓重算阴影。只用公开 API，不需要 Tauri 的 `macos-private-api`。
/// 可重复调用；窗口显示后再调一次，阴影才会贴合圆角。必须在主线程调用。
///
/// # Safety
/// `ns_window` 必须是有效的 `NSWindow` 指针（Tauri 的 `ns_window()`）。
#[cfg(target_os = "macos")]
pub unsafe fn round_corners(ns_window: *mut std::ffi::c_void, radius: f64) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSColor, NSWindow};
    if ns_window.is_null() {
        return;
    }
    // SAFETY: 调用方保证指针有效且在主线程。
    let window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    let Some(view) = window.contentView() else {
        return;
    };
    view.setWantsLayer(true);
    // SAFETY: 层托管的视图一定有 CALayer；这两个都是 CALayer 的公开属性。
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        if let Some(layer) = layer.as_ref() {
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
    }
    window.invalidateShadow();
}
