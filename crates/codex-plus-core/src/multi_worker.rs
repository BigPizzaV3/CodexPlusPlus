#[cfg(any(windows, test))]
use std::path::Path;

#[cfg(any(windows, test))]
pub(crate) fn worker_identity_matches(
    expected_birth: Option<u64>,
    actual_birth: Option<u64>,
    arguments: &[String],
    options_path: &Path,
) -> bool {
    // 旧清单没有创建身份，或者 PID 已复用时，宁可留下 worker 也不猜测归属。
    if expected_birth.is_none()
        || expected_birth != actual_birth
        || arguments.len() != 3
        || arguments[1] != "--codex-plus-multi-worker"
    {
        return false;
    }
    let Ok(expected) = options_path.canonicalize() else {
        return false;
    };
    let Ok(actual) = Path::new(&arguments[2]).canonicalize() else {
        return false;
    };
    expected == actual
}

/// 直接从仍由父进程持有的 Child 句柄读取创建身份，避免按 PID 重新打开另一进程。
pub fn worker_birth_id(child: &std::process::Child) -> Option<u64> {
    #[cfg(windows)]
    {
        crate::windows_integration::child_process_birth_id(child)
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        None
    }
}

#[cfg(windows)]
pub fn terminate_verified_worker(
    pid: u32,
    options_path: &Path,
    expected_birth: Option<u64>,
) -> anyhow::Result<bool> {
    crate::windows_integration::terminate_verified_multi_worker(pid, options_path, expected_birth)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(options: &Path) -> Vec<String> {
        vec![
            "CodexPlusPlus.exe".into(),
            "--codex-plus-multi-worker".into(),
            options.to_string_lossy().into_owned(),
        ]
    }

    #[test]
    fn worker_identity_requires_birth_marker_and_exact_options_file() {
        let temp = tempfile::tempdir().unwrap();
        let options = temp.path().join("instance A worker-options.json");
        std::fs::write(&options, "{}").unwrap();
        let args = arguments(&options);
        assert!(worker_identity_matches(Some(41), Some(41), &args, &options));
        assert!(!worker_identity_matches(None, Some(41), &args, &options));
        assert!(!worker_identity_matches(Some(41), None, &args, &options));
        assert!(!worker_identity_matches(
            Some(41),
            Some(42),
            &args,
            &options
        ));

        let mut wrong_marker = args.clone();
        wrong_marker[1] = "--codex-plus-multi-worker-other".into();
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &wrong_marker,
            &options
        ));
        let mut extra_arg = args.clone();
        extra_arg.push("--unexpected".into());
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &extra_arg,
            &options
        ));
    }

    #[test]
    fn worker_identity_rejects_neighbor_missing_and_prefix_only_paths() {
        let temp = tempfile::tempdir().unwrap();
        let options = temp.path().join("worker-options.json");
        let neighbor = temp.path().join("worker-options.json.backup");
        std::fs::write(&options, "{}").unwrap();
        std::fs::write(&neighbor, "{}").unwrap();
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &arguments(&neighbor),
            &options
        ));
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &arguments(&options),
            &neighbor
        ));
        let missing = temp.path().join("missing.json");
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &arguments(&missing),
            &missing
        ));
    }
}
