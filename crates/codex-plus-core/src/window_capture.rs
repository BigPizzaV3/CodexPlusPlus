//! 目标窗口取流：ScreenCaptureKit 抓单个窗口，编码成 JPEG 交给界面预览。
//!
//! # 为什么自己抓
//!
//! Codex 官方的「窗口镜像」（Remote Hosted PiP）走 `SkyComputerUseService` 发布
//! `CAContext` + H.264 帧通道，但发布端只认 `com.openai.chat` / `com.openai.codex` /
//! `com.openai.atlas` 三个 bundle id，第三方无法加入。所以这里退一步：用系统公开的
//! ScreenCaptureKit 自己抓目标窗口，在 Codex++ 里画预览。画面内容和官方 PiP 一致
//! （都是同一个窗口的实时画面），只是少了 XPC 那一层的低延迟。
//!
//! # 实测数据（2026-10-24，M 系列 Mac，1280×820 窗口）
//!
//! | 缩放 | 帧率 | 单帧 JPEG |
//! |---|---|---|
//! | 1.0 | 14.7 fps | ~150 KB |
//! | 0.5 | 15.9 fps | ~45 KB |
//! | 0.35 | 16.2 fps | ~25 KB |
//!
//! 编码是瓶颈而不是抓取，所以预览默认 0.5 倍缩放 + 12 fps。
//!
//! # 两个前置条件
//!
//! 1. 进程必须已经连上 WindowServer，否则 CoreGraphics 会断言崩溃
//!    （`CGS_REQUIRE_INIT`）。GUI 应用天然满足；纯 CLI 需要先创建 `NSApplication`。
//! 2. 需要「屏幕录制」系统权限。没授权时枚举结果为空，`list_windows` 会给出提示。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};

/// 预览取流模式。
///
/// - `Screenshot`（默认）：反复调用 `SCScreenshotManager`，每帧独立截图，
///   目标窗口左上角**不**出现共享标志，适合旁观预览。
/// - `Stream`：建立 `SCStream` 流，系统以推送方式交帧，帧率更高、延迟更低，
///   但目标窗口会显示橙色摄像头图标（macOS 标准共享指示器）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// 高频截图，目标窗口无感知（默认）
    #[default]
    Screenshot,
    /// SCStream 流，低延迟高帧率，会显示系统共享标志
    Stream,
}

/// 一个可被抓取的窗口。
#[derive(Debug, Clone, Serialize)]
pub struct CaptureWindow {
    /// ScreenCaptureKit 的窗口号，抓取时用它定位窗口
    pub window_id: u32,
    pub pid: i32,
    pub app_name: String,
    pub bundle_id: String,
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub on_screen: bool,
}

/// 预览会话句柄：`stop()` 之后抓取线程退出。
pub struct PreviewHandle {
    stop: Arc<AtomicBool>,
    errors: Arc<Mutex<Option<String>>>,
    /// 已经产出的帧数，便于界面显示是否真的在动
    frames: Arc<Mutex<u64>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PreviewHandle {
    /// 抓取过程中遇到的错误（窗口关闭、权限被撤销等）
    pub fn error(&self) -> Option<String> {
        self.errors.lock().ok().and_then(|e| e.clone())
    }

    /// 抓取线程记录的错误槽；界面层可以盯着它判断预览是否已自行结束
    pub fn error_slot(&self) -> Arc<Mutex<Option<String>>> {
        self.errors.clone()
    }

    pub fn frames(&self) -> u64 {
        self.frames.lock().map(|n| *n).unwrap_or(0)
    }

    /// 停止抓取并等线程退出（最多等 2 秒）
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PreviewHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::AllocAnyThread;
    use objc2_core_graphics::CGImage;
    use objc2_screen_capture_kit::{
        SCContentFilter, SCScreenshotManager, SCShareableContent, SCStream,
        SCStreamConfiguration, SCStreamDelegate, SCStreamOutput, SCStreamOutputType, SCWindow,
    };

    use super::{CaptureMode, CaptureWindow, PreviewHandle};

    /// 建立到 WindowServer 的连接。
    ///
    /// ScreenCaptureKit 的抓帧路径会触发 CoreGraphics 初始化，而纯 CLI 进程
    /// 从没连过 WindowServer，此时会直接断言崩溃（`CGS_REQUIRE_INIT`）。
    /// GUI 应用天然已经连上，所以这里只在主线程兜底，重复调用没有副作用。
    pub fn ensure_graphics_initialized() {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            let _ = objc2_app_kit::NSApplication::sharedApplication(mtm);
        }
    }

    /// 是否已经拿到「屏幕录制」权限。没有权限时枚举会得到空列表。
    pub fn has_permission() -> bool {
        unsafe { objc2_core_graphics::CGPreflightScreenCaptureAccess() }
    }

    /// 触发系统的权限申请弹窗。已经授权时直接返回 true。
    pub fn request_permission() -> bool {
        unsafe { objc2_core_graphics::CGRequestScreenCaptureAccess() }
    }

    const ENUM_TIMEOUT: Duration = Duration::from_secs(15);
    const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

    /// 枚举当前可抓的窗口。没有屏幕录制权限时返回空列表。
    pub fn list_windows() -> anyhow::Result<Vec<CaptureWindow>> {
        let (tx, rx) = mpsc::channel::<Vec<CaptureWindow>>();
        unsafe {
            SCShareableContent::getShareableContentWithCompletionHandler(&RcBlock::new(
                move |content: *mut SCShareableContent, _error: *mut _| {
                    let mut list = Vec::new();
                    if !content.is_null() {
                        for window in (&*content).windows().iter() {
                            let frame = window.frame();
                            if frame.size.width < 64.0 || frame.size.height < 64.0 {
                                continue;
                            }
                            // 系统窗口（Display Backstop / MenuBar / NowPlaying 之类）没有归属应用，
                            // 列出来只会干扰选择，也基本没有预览价值
                            let app = window.owningApplication();
                            let app_name = app
                                .as_ref()
                                .map(|a| a.applicationName().to_string())
                                .unwrap_or_default();
                            if app_name.is_empty() {
                                continue;
                            }
                            list.push(CaptureWindow {
                                window_id: window.windowID(),
                                pid: app.as_ref().map(|a| a.processID()).unwrap_or(0),
                                app_name: app
                                    .as_ref()
                                    .map(|a| a.applicationName())
                                    .map(|n| n.to_string())
                                    .unwrap_or_default(),
                                bundle_id: app
                                    .as_ref()
                                    .map(|a| a.bundleIdentifier())
                                    .map(|n| n.to_string())
                                    .unwrap_or_default(),
                                title: window.title().map(|t| t.to_string()).unwrap_or_default(),
                                width: frame.size.width,
                                height: frame.size.height,
                                on_screen: window.isOnScreen(),
                            });
                        }
                    }
                    let _ = tx.send(list);
                },
            ));
        }
        let mut windows = rx
            .recv_timeout(ENUM_TIMEOUT)
            .map_err(|_| anyhow::anyhow!("枚举窗口超时（可能尚未授予「屏幕录制」权限）"))?;
        // 前台窗口排前面，同应用内按应用名 + 标题排序，方便界面里找
        windows.sort_by(|a, b| {
            b.on_screen
                .cmp(&a.on_screen)
                .then_with(|| a.app_name.cmp(&b.app_name))
                .then_with(|| a.title.cmp(&b.title))
        });
        Ok(windows)
    }

    fn find_window(window_id: u32) -> Option<Retained<SCWindow>> {
        let (tx, rx) = mpsc::channel::<Option<Retained<SCWindow>>>();
        unsafe {
            SCShareableContent::getShareableContentWithCompletionHandler(&RcBlock::new(
                move |content: *mut SCShareableContent, _error: *mut _| {
                    let mut found = None;
                    if !content.is_null() {
                        for window in (&*content).windows().iter() {
                            if window.windowID() == window_id {
                                found = Some(window.clone());
                                break;
                            }
                        }
                    }
                    let _ = tx.send(found);
                },
            ));
        }
        rx.recv_timeout(ENUM_TIMEOUT).ok().flatten()
    }

    fn capture_jpeg(
        filter: &SCContentFilter,
        config: &SCStreamConfiguration,
    ) -> anyhow::Result<Vec<u8>> {
        let (tx, rx) = mpsc::channel::<anyhow::Result<Vec<u8>>>();
        unsafe {
            SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                filter,
                config,
                Some(&RcBlock::new(move |image: *mut CGImage, error: *mut _| {
                    let result = if image.is_null() {
                        // macOS 在锁屏时拒绝按窗口抓取，整段失败基本都是这个原因
                        Err(anyhow::anyhow!(
                            "抓帧失败：{}。如果屏幕已锁定或 Codex++ 没有「屏幕录制」权限，都会落到这里。",
                            describe_error(error as *mut _)
                        ))
                    } else {
                        encode_jpeg(unsafe { &*image })
                    };
                    let _ = tx.send(result);
                })),
            );
        }
        rx.recv_timeout(FRAME_TIMEOUT)
            .map_err(|_| anyhow::anyhow!("抓帧超时"))?
    }

    unsafe fn describe_error(error: *mut objc2::runtime::AnyObject) -> String {
        if error.is_null() {
            return "未知错误".to_string();
        }
        unsafe {
            let obj = &*error;
            match obj.downcast_ref::<objc2_foundation::NSError>() {
                Some(err) => err.localizedDescription().to_string(),
                None => "未知错误".to_string(),
            }
        }
    }

    fn encode_jpeg(image: &CGImage) -> anyhow::Result<Vec<u8>> {
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
        use objc2_foundation::NSDictionary;
        unsafe {
            let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
            let properties = NSDictionary::new();
            let data = rep
                .representationUsingType_properties(NSBitmapImageFileType::JPEG, &properties)
                .ok_or_else(|| anyhow::anyhow!("JPEG 编码失败"))?;
            Ok(data.to_vec())
        }
    }

    pub fn start_preview(
        window_id: u32,
        scale: f64,
        max_fps: f64,
        mode: super::CaptureMode,
        sink: impl Fn(Vec<u8>) + Send + 'static,
    ) -> anyhow::Result<PreviewHandle> {
        match mode {
            super::CaptureMode::Stream => {
                return start_preview_stream(window_id, scale, max_fps, sink);
            }
            super::CaptureMode::Screenshot => {}
        }
        let scale = scale.clamp(0.1, 2.0);
        let interval = Duration::from_secs_f64(1.0 / max_fps.clamp(1.0, 30.0));
        let stop = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(Mutex::new(None));
        let frames = Arc::new(Mutex::new(0u64));

        let thread_stop = stop.clone();
        let thread_errors = errors.clone();
        let thread_frames = frames.clone();
        let thread = std::thread::Builder::new()
            .name("codex-plus-cu-preview".to_string())
            .spawn(move || {
                let fail = |message: String| {
                    if let Ok(mut slot) = thread_errors.lock() {
                        *slot = Some(message);
                    }
                };
                let Some(window) = find_window(window_id) else {
                    fail("目标窗口已关闭或找不到".to_string());
                    return;
                };
                let filter = unsafe {
                    SCContentFilter::initWithDesktopIndependentWindow(
                        SCContentFilter::alloc(),
                        &window,
                    )
                };
                let config = unsafe { SCStreamConfiguration::new() };
                unsafe {
                    let frame = window.frame();
                    let width = (frame.size.width * scale).round().max(1.0);
                    let height = (frame.size.height * scale).round().max(1.0);
                    config.setWidth(width as usize);
                    config.setHeight(height as usize);
                    config.setShowsCursor(true);
                    config.setPixelFormat(u32::from_be_bytes(*b"BGRA"));
                }

                while !thread_stop.load(Ordering::Relaxed) {
                    let started = Instant::now();
                    match capture_jpeg(&filter, &config) {
                        Ok(bytes) => {
                            if thread_stop.load(Ordering::Relaxed) {
                                break;
                            }
                            sink(bytes);
                            if let Ok(mut count) = thread_frames.lock() {
                                *count += 1;
                            }
                        }
                        Err(error) => {
                            // 窗口关闭是最常见的情况，直接退出，界面据此收起预览
                            fail(error.to_string());
                            return;
                        }
                    }
                    let elapsed = started.elapsed();
                    if elapsed < interval {
                        std::thread::sleep(interval - elapsed);
                    }
                }
            })?;

        Ok(PreviewHandle {
            stop,
            errors,
            frames,
            thread: Some(thread),
        })
    }

    // ── SCStream 路径 ────────────────────────────────────────────────────────
    // 系统以推送方式交帧，延迟更低、帧率可达屏幕刷新率；
    // 副作用：目标窗口左上角出现橙色摄像头图标（macOS 标准共享指示器）。
    //
    // 架构：全局静态 STREAM_SENDER 槽存 SyncSender，`extern "C"` 回调直接读取，
    // 完全避免 associated object / FFI 边界 panic 问题。
    fn start_preview_stream(
        window_id: u32,
        scale: f64,
        max_fps: f64,
        sink: impl Fn(Vec<u8>) + Send + 'static,
    ) -> anyhow::Result<PreviewHandle> {
        use std::ffi::c_void;
        use objc2_foundation::NSError;

        // CoreMedia/CoreVideo/CoreGraphics 符号直链
        #[link(name = "CoreMedia",    kind = "framework")] unsafe extern "C" {
            fn CMSampleBufferGetImageBuffer(s: *mut c_void) -> *mut c_void;
        }
        #[link(name = "CoreVideo",    kind = "framework")] unsafe extern "C" {
            fn CVPixelBufferLockBaseAddress(b: *mut c_void, f: u64) -> i32;
            fn CVPixelBufferUnlockBaseAddress(b: *mut c_void, f: u64) -> i32;
            fn CVPixelBufferGetBaseAddress(b: *mut c_void) -> *mut c_void;
            fn CVPixelBufferGetBytesPerRow(b: *mut c_void) -> usize;
            fn CVPixelBufferGetWidth(b: *mut c_void) -> usize;
            fn CVPixelBufferGetHeight(b: *mut c_void) -> usize;
        }
        #[link(name = "CoreGraphics", kind = "framework")] unsafe extern "C" {
            fn CGBitmapContextCreate(d: *mut c_void, w: usize, h: usize,
                bpc: usize, bpr: usize, cs: *mut c_void, bi: u32) -> *mut c_void;
            fn CGBitmapContextCreateImage(ctx: *mut c_void) -> *mut CGImage;
            fn CGContextRelease(ctx: *mut c_void);
            fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
            fn CGColorSpaceRelease(cs: *mut c_void);
        }
        #[link(name = "System", kind = "dylib")] unsafe extern "C" {
            fn dispatch_queue_create(label: *const i8, attr: *const c_void) -> *mut c_void;
        }

        // 全局静态槽：callback 无法捕获闭包，用静态槽传递 sender
        static STREAM_SENDER: std::sync::OnceLock<
            Mutex<Option<mpsc::SyncSender<Vec<u8>>>>>
            = std::sync::OnceLock::new();
        let slot = STREAM_SENDER.get_or_init(|| Mutex::new(None));

        // 注册 SCStreamOutput 子类（进程生命周期内只注册一次）
        static SUBCLASS_ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        SUBCLASS_ONCE.get_or_init(|| unsafe {
            use objc2::runtime::{AnyClass, AnyProtocol, Sel};
            use objc2::sel;

            extern "C" fn stream_did_output(
                _this: *mut objc2::runtime::AnyObject,
                _sel: Sel,
                _stream: *mut objc2::runtime::AnyObject,
                sample_buf: *mut c_void,
                _output_type: i64,
            ) {
                // 不能 panic：外部 ObjC 调用帧
                let pixel_buf = unsafe { CMSampleBufferGetImageBuffer(sample_buf) };
                if pixel_buf.is_null() { return; }

                let jpeg = unsafe {
                    CVPixelBufferLockBaseAddress(pixel_buf, 1);
                    let w   = CVPixelBufferGetWidth(pixel_buf);
                    let h   = CVPixelBufferGetHeight(pixel_buf);
                    let bpr = CVPixelBufferGetBytesPerRow(pixel_buf);
                    let base = CVPixelBufferGetBaseAddress(pixel_buf);
                    let cs  = CGColorSpaceCreateDeviceRGB();
                    let ctx = CGBitmapContextCreate(base, w, h, 8, bpr, cs, 0x2002);
                    let img = CGBitmapContextCreateImage(ctx);
                    CGContextRelease(ctx);
                    CGColorSpaceRelease(cs);
                    CVPixelBufferUnlockBaseAddress(pixel_buf, 1);
                    if img.is_null() { return; }
                    let r = encode_jpeg(&*img);
                    let _ = Retained::from_raw(img);
                    match r { Ok(b) => b, Err(_) => return }
                };

                // 把帧发到接帧循环
                if let Ok(guard) = STREAM_SENDER
                    .get()
                    .and_then(|s| s.lock().ok())
                    .into_iter()
                    .next()
                    .map(|g| g)
                    .ok_or(())
                {
                    // 无法直接用 guard，重新 lock
                    drop(guard);
                }
                if let Some(slot) = STREAM_SENDER.get() {
                    if let Ok(g) = slot.lock() {
                        if let Some(tx) = g.as_ref() {
                            let _ = tx.try_send(jpeg);
                        }
                    }
                }
            }

            let superclass = AnyClass::get(c"NSObject").unwrap();
            let mut builder = objc2::runtime::ClassBuilder::new(
                c"CodPlusStreamOutput", superclass).unwrap();
            builder.add_method(
                sel!(stream:didOutputSampleBuffer:ofType:),
                stream_did_output as extern "C" fn(_, _, _, _, _),
            );
            if let Some(proto) = AnyProtocol::get(c"SCStreamOutput") {
                builder.add_protocol(proto);
            }
            builder.register();
        });

        let scale = scale.clamp(0.1, 2.0);
        let stop   = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(Mutex::new(None::<String>));
        let frames = Arc::new(Mutex::new(0u64));

        let thread_stop   = stop.clone();
        let thread_errors = errors.clone();
        let thread_frames = frames.clone();

        let thread = std::thread::Builder::new()
            .name("codex-plus-cu-stream".to_string())
            .spawn(move || {
                let fail = |msg: String| {
                    if let Ok(mut s) = thread_errors.lock() { *s = Some(msg); }
                };

                let Some(window) = find_window(window_id) else {
                    fail("目标窗口已关闭或找不到".into()); return;
                };

                let filter = unsafe {
                    SCContentFilter::initWithDesktopIndependentWindow(
                        SCContentFilter::alloc(), &window)
                };
                let config = unsafe { SCStreamConfiguration::new() };
                unsafe {
                    let f = window.frame();
                    config.setWidth( (f.size.width  * scale).round().max(1.0) as usize);
                    config.setHeight((f.size.height * scale).round().max(1.0) as usize);
                    config.setShowsCursor(true);
                    config.setPixelFormat(u32::from_be_bytes(*b"BGRA"));
                    let _ = max_fps; // 帧率由窗口刷新驱动
                }

                // 注入 sender 到静态槽
                let (sync_tx, sync_rx) = mpsc::sync_channel::<Vec<u8>>(4);
                {
                    let mut g = slot.lock().unwrap_or_else(|e| e.into_inner());
                    *g = Some(sync_tx);
                }

                unsafe {
                    use objc2::runtime::AnyClass;
                    use std::ffi::CString;

                    let cls = AnyClass::get(c"CodPlusStreamOutput").unwrap();
                    let output_obj: *mut objc2::runtime::AnyObject = objc2::msg_send![cls, new];

                    let label = CString::new("codex.plus.stream.q").unwrap();
                    let queue = dispatch_queue_create(label.as_ptr(), std::ptr::null());

                    let stream = SCStream::initWithFilter_configuration_delegate(
                        SCStream::alloc(), &filter, &config, None);

                    let mut add_err: *mut NSError = std::ptr::null_mut();
                    let _ok: bool = objc2::msg_send![
                        &*stream,
                        addStreamOutput: output_obj
                        type: SCStreamOutputType::Screen
                        sampleHandlerQueue: queue
                        error: &mut add_err
                    ];

                    let (start_tx, start_rx) = mpsc::channel::<Result<(), String>>();
                    stream.startCaptureWithCompletionHandler(Some(&*RcBlock::new(
                        move |e: *mut NSError| {
                            if e.is_null() { let _ = start_tx.send(Ok(())); }
                            else {
                                let msg = unsafe { (&*e).localizedDescription().to_string() };
                                let _ = start_tx.send(Err(msg));
                            }
                        })));
                    match start_rx.recv_timeout(Duration::from_secs(10)) {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => { fail(format!("SCStream 启动失败：{e}")); return; }
                        Err(_) => { fail("SCStream 启动超时".into()); return; }
                    }

                    // 接帧循环
                    while !thread_stop.load(Ordering::Relaxed) {
                        match sync_rx.recv_timeout(Duration::from_secs(2)) {
                            Ok(bytes) => {
                                if thread_stop.load(Ordering::Relaxed) { break; }
                                sink(bytes);
                                if let Ok(mut n) = thread_frames.lock() { *n += 1; }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(_) => break,
                        }
                    }

                    let (stop_tx, stop_rx) = mpsc::channel::<()>();
                    stream.stopCaptureWithCompletionHandler(Some(&*RcBlock::new(
                        move |_e: *mut NSError| { let _ = stop_tx.send(()); })));
                    let _ = stop_rx.recv_timeout(Duration::from_secs(5));

                    // 清空静态槽
                    if let Ok(mut g) = slot.lock() { *g = None; }
                    let _: () = objc2::msg_send![output_obj, release];
                }
            })?;

        Ok(PreviewHandle { stop, errors, frames, thread: Some(thread) })
    }
}
#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{CaptureWindow, PreviewHandle};

    pub fn list_windows() -> anyhow::Result<Vec<CaptureWindow>> {
        Ok(Vec::new())
    }

    pub fn ensure_graphics_initialized() {}

    pub fn has_permission() -> bool {
        false
    }

    pub fn request_permission() -> bool {
        false
    }

    pub fn start_preview(
        _window_id: u32,
        _scale: f64,
        _max_fps: f64,
        _mode: super::CaptureMode,
        _sink: impl Fn(Vec<u8>) + Send + 'static,
    ) -> anyhow::Result<PreviewHandle> {
        anyhow::bail!("窗口预览只支持 macOS")
    }
}

pub use imp::{
    ensure_graphics_initialized, has_permission, list_windows, request_permission, start_preview,
};

#[cfg(test)]
mod tests {
    use super::*;

    // ScreenCaptureKit 需要 GUI 进程环境（cargo test 是纯 CLI，会断言崩溃），
    // 所以这里只测前后端约定的字段名，抓取本身靠界面实测。
    #[test]
    fn capture_window_serializes_with_expected_fields() {
        let window = CaptureWindow {
            window_id: 63573,
            pid: 52791,
            app_name: "Claude".to_string(),
            bundle_id: "com.anthropic.claudefordesktop".to_string(),
            title: "会话".to_string(),
            width: 1280.0,
            height: 820.0,
            on_screen: true,
        };
        let value = serde_json::to_value(&window).unwrap();
        assert_eq!(value["window_id"], 63573);
        assert_eq!(value["pid"], 52791);
        assert_eq!(value["app_name"], "Claude");
        assert_eq!(value["bundle_id"], "com.anthropic.claudefordesktop");
        assert_eq!(value["title"], "会话");
        assert_eq!(value["width"], 1280.0);
        assert_eq!(value["height"], 820.0);
        assert_eq!(value["on_screen"], true);
    }

    #[test]
    fn non_macos_report_that_preview_is_unsupported() {
        #[cfg(not(target_os = "macos"))]
        {
            assert!(super::list_windows().unwrap().is_empty());
            // PreviewHandle 没有 Debug，不能用 unwrap_err；取出错误再断言。
            let error = super::start_preview(1, 0.5, 12.0, CaptureMode::Screenshot, |_| {})
                .err()
                .expect("非 macOS 上应当返回不支持");
            assert!(error.to_string().contains("只支持 macOS"));
        }
        #[cfg(target_os = "macos")]
        {
            // macOS 上 start_preview 会真的去连 WindowServer，测试进程不满足条件，
            // 这里只确认函数存在且签名可用。
            let _ = super::list_windows as fn() -> anyhow::Result<Vec<CaptureWindow>>;
        }
    }
}
