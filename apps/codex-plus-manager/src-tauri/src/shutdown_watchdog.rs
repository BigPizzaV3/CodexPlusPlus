use std::time::Duration;

/// 日志只尽力后台写入；不等待写盘，线程创建失败也不能妨碍退出。
pub(crate) fn record_best_effort(record: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("shutdown-diagnostic".into())
        .spawn(record);
}

/// 只有进程真正退出才解除兜底，不能用后台清理完成代替退出确认。
pub(crate) fn arm(timeout: Duration, record_timeout: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        // 诊断写盘也可能阻塞，不能让日志阻止最终退出。
        let (recorded, completion) = std::sync::mpsc::channel();
        record_best_effort(move || {
            record_timeout();
            let _ = recorded.send(());
        });
        let _ = completion.recv_timeout(Duration::from_millis(250));
        std::process::exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    #[ignore = "subprocess fixture; invoked by watchdog_covers_the_entire_exit"]
    fn watchdog_child() {
        let mode = std::env::var("CODEX_PLUS_SHUTDOWN_WATCHDOG_TEST").unwrap();
        let cleanup_complete = AtomicBool::new(mode == "complete");
        let block_diagnostic = mode == "blocked_diagnostic";
        if mode == "blocked_entry" {
            record_best_effort(|| {
                loop {
                    std::thread::park();
                }
            });
        }
        arm(Duration::from_millis(100), move || {
            if block_diagnostic {
                loop {
                    std::thread::park();
                }
            }
            println!(
                "cleanupComplete={}",
                cleanup_complete.load(Ordering::SeqCst)
            );
        });
        if mode == "normal_exit" {
            std::process::exit(23);
        }
        // 模拟最终退出事件未生效或清理阻塞；不能让测试框架自行退出掩盖问题。
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn watchdog_covers_the_entire_exit() {
        for mode in [
            "incomplete",
            "complete",
            "normal_exit",
            "blocked_diagnostic",
            "blocked_entry",
        ] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "shutdown_watchdog::tests::watchdog_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("CODEX_PLUS_SHUTDOWN_WATCHDOG_TEST", mode)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let mut child = ChildGuard(command.spawn().unwrap());
            let deadline = Instant::now() + Duration::from_secs(5);
            let status = loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    Instant::now() < deadline,
                    "watchdog did not terminate {mode}"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(
                status.code(),
                Some(if mode == "normal_exit" { 23 } else { 0 }),
                "{mode}"
            );
            use std::io::Read;
            let mut stdout = String::new();
            child
                .0
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut stdout)
                .unwrap();
            if !matches!(mode, "normal_exit" | "blocked_diagnostic") {
                assert!(
                    stdout.contains(&format!("cleanupComplete={}", mode == "complete")),
                    "{stdout}"
                );
            } else {
                assert!(!stdout.contains("cleanupComplete="), "{stdout}");
            }
        }
    }

    #[test]
    fn entry_logging_returns_before_a_blocked_writer_finishes() {
        let (writer_started, writer_started_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let (writer_finished, writer_finished_rx) = std::sync::mpsc::channel();
        let (caller_returned, caller_returned_rx) = std::sync::mpsc::channel();
        let caller = std::thread::spawn(move || {
            record_best_effort(move || {
                writer_started.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(5));
                writer_finished.send(()).unwrap();
            });
            caller_returned.send(()).unwrap();
        });
        let writer_started = writer_started_rx.recv_timeout(Duration::from_secs(2));
        let caller_returned = caller_returned_rx.recv_timeout(Duration::from_secs(1));
        let _ = release.send(());
        caller.join().unwrap();
        assert!(writer_started.is_ok(), "diagnostic writer never started");
        assert!(caller_returned.is_ok(), "caller waited for diagnostic I/O");
        writer_finished_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}
