//! 以「放弃责任归属」的方式拉起子进程（macOS）。
//!
//! # 为什么需要
//!
//! macOS 给每个进程记一个 responsible process（TCC 授权与代码签名校验都按它算）。
//! 默认情况下子进程继承父进程的责任进程：从终端起，责任进程是终端；从 Claude 的
//! `disclaimer --pgroup` 起，责任进程就是被拉起的程序自己。
//!
//! Codex 的原生 CU 客户端会校验这条链上的责任进程签名（`_senderResponsibleTeamID`
//! 等字段）。实测（2026-10-24）：
//!
//! | 谁拉起 Codex++ | 责任进程 | 结果 |
//! |---|---|---|
//! | 终端 / Claude.app 里的 shell | Claude.app（正式签名） | ✅ |
//! | Claude 会话（经 `disclaimer`） | Codex++（本地构建是 ad-hoc，无 Team ID） | ❌ `bootstrapTimedOut` |
//! | 管理器 GUI 自检 | Codex++ | ❌ `bootstrapTimedOut` |
//!
//! 只要让 OpenAI 签名的 `cua_node/bin/node` 成为**自己的**责任进程，三种情况都能通过。
//! 这里用的 `responsibility_spawnattrs_setdisclaim` 是 libSystem 导出的符号，
//! Chromium、LLDB 与 Claude 自己的 disclaimer 都用它。
//!
//! 其它平台没有这个概念，退回普通 `Command`。

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::Path;

/// 被拉起的子进程：标准输入输出各一条管道，标准错误继承父进程。
pub struct DisclaimedChild {
    pub stdin: Option<Box<dyn Write + Send>>,
    pub stdout: Option<Box<dyn Read + Send>>,
    inner: Inner,
}

enum Inner {
    #[cfg(target_os = "macos")]
    Spawned { pid: i32, reaped: bool },
    #[allow(dead_code)]
    Command(std::process::Child),
}

impl DisclaimedChild {
    pub fn id(&self) -> u32 {
        match &self.inner {
            #[cfg(target_os = "macos")]
            Inner::Spawned { pid, .. } => *pid as u32,
            Inner::Command(child) => child.id(),
        }
    }

    pub fn kill(&mut self) {
        match &mut self.inner {
            #[cfg(target_os = "macos")]
            Inner::Spawned { pid, reaped } => {
                if !*reaped {
                    // SAFETY: 只给自己拉起、尚未回收的 pid 发信号。
                    unsafe { macos::kill(*pid, macos::SIGKILL) };
                }
            }
            Inner::Command(child) => {
                let _ = child.kill();
            }
        }
    }

    pub fn wait(&mut self) {
        match &mut self.inner {
            #[cfg(target_os = "macos")]
            Inner::Spawned { pid, reaped } => {
                if !*reaped {
                    let mut status = 0;
                    // SAFETY: 回收自己的子进程，避免留下僵尸。
                    unsafe { macos::waitpid(*pid, &mut status, 0) };
                    *reaped = true;
                }
            }
            Inner::Command(child) => {
                let _ = child.wait();
            }
        }
    }
}

impl Drop for DisclaimedChild {
    fn drop(&mut self) {
        self.kill();
        self.wait();
    }
}

/// 拉起 `program args…`，额外设置 `envs`（在继承的环境之上覆盖）。
pub fn spawn(
    program: &Path,
    args: &[&OsStr],
    envs: &[(String, String)],
) -> anyhow::Result<DisclaimedChild> {
    let mut merged: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (key, value) in envs {
        merged.insert(key.into(), value.into());
    }
    spawn_with_env(program, args, merged)
}

/// 拉起 `program args…`，环境**只有** `envs`，不继承当前进程的任何变量。
///
/// 给第三方程序用：调用方（例如 Claude Code）的环境里可能有 API Key 之类的东西，
/// 对方用不到就不该拿到。
pub fn spawn_clean(
    program: &Path,
    args: &[&OsStr],
    envs: &[(String, String)],
) -> anyhow::Result<DisclaimedChild> {
    let env = envs
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect();
    spawn_with_env(program, args, env)
}

fn spawn_with_env(
    program: &Path,
    args: &[&OsStr],
    env: BTreeMap<OsString, OsString>,
) -> anyhow::Result<DisclaimedChild> {
    #[cfg(target_os = "macos")]
    {
        macos::spawn(program, args, &env)
    }
    #[cfg(not(target_os = "macos"))]
    {
        use anyhow::Context;
        use std::process::{Command, Stdio};
        let mut command = Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(&env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(crate::windows_create_no_window());
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("无法启动 {}", program.display()))?;
        let stdin = child.stdin.take().map(|pipe| Box::new(pipe) as Box<dyn Write + Send>);
        let stdout = child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>);
        Ok(DisclaimedChild {
            stdin,
            stdout,
            inner: Inner::Command(child),
        })
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::BTreeMap;
    use std::ffi::{CString, OsStr, OsString, c_char, c_int, c_short, c_void};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use anyhow::{Context, bail};

    use super::{DisclaimedChild, Inner};

    // macOS 上这两个类型都是不透明指针（`typedef void *posix_spawnattr_t;`）。
    type PosixSpawnAttr = *mut c_void;
    type PosixSpawnFileActions = *mut c_void;

    pub(super) const SIGKILL: c_int = 9;
    const STDIN: c_int = 0;
    const STDOUT: c_int = 1;

    unsafe extern "C" {
        fn posix_spawnattr_init(attr: *mut PosixSpawnAttr) -> c_int;
        fn posix_spawnattr_destroy(attr: *mut PosixSpawnAttr) -> c_int;
        fn posix_spawnattr_setflags(attr: *mut PosixSpawnAttr, flags: c_short) -> c_int;
        fn posix_spawn_file_actions_init(actions: *mut PosixSpawnFileActions) -> c_int;
        fn posix_spawn_file_actions_destroy(actions: *mut PosixSpawnFileActions) -> c_int;
        fn posix_spawn_file_actions_adddup2(
            actions: *mut PosixSpawnFileActions,
            fd: c_int,
            new_fd: c_int,
        ) -> c_int;
        fn posix_spawn(
            pid: *mut i32,
            path: *const c_char,
            actions: *const PosixSpawnFileActions,
            attr: *const PosixSpawnAttr,
            argv: *const *const c_char,
            envp: *const *const c_char,
        ) -> c_int;
        /// libSystem 导出；声明见 Chromium `base/process/launch_mac.cc`。
        fn responsibility_spawnattrs_setdisclaim(attr: *mut PosixSpawnAttr, disclaim: c_int)
        -> c_int;
        pub(super) fn waitpid(pid: i32, status: *mut c_int, options: c_int) -> i32;
        pub(super) fn kill(pid: i32, signal: c_int) -> c_int;
    }

    /// 新进程独立成一个进程组，父进程收到的终端信号不会误伤它，
    /// 收尾时由 `DisclaimedChild` 显式 kill。
    const POSIX_SPAWN_SETPGROUP: c_short = 0x0002;

    fn cstring(value: &OsStr) -> anyhow::Result<CString> {
        CString::new(value.as_bytes()).context("参数里含有 NUL 字节")
    }

    /// `env` 是子进程的完整环境，这里不再合并当前进程的变量。
    pub(super) fn spawn(
        program: &Path,
        args: &[&OsStr],
        env: &BTreeMap<OsString, OsString>,
    ) -> anyhow::Result<DisclaimedChild> {
        let (stdin_read, stdin_write) = std::io::pipe().context("无法创建标准输入管道")?;
        let (stdout_read, stdout_write) = std::io::pipe().context("无法创建标准输出管道")?;

        let path = cstring(program.as_os_str())?;
        let mut argv_owned = vec![path.clone()];
        for arg in args {
            argv_owned.push(cstring(arg)?);
        }
        let mut argv: Vec<*const c_char> = argv_owned.iter().map(|value| value.as_ptr()).collect();
        argv.push(std::ptr::null());

        let mut env_owned = Vec::with_capacity(env.len());
        for (key, value) in env {
            let mut pair = key.as_bytes().to_vec();
            pair.push(b'=');
            pair.extend_from_slice(value.as_bytes());
            if let Ok(pair) = CString::new(pair) {
                env_owned.push(pair);
            }
        }
        let mut envp: Vec<*const c_char> = env_owned.iter().map(|value| value.as_ptr()).collect();
        envp.push(std::ptr::null());

        let mut attr: PosixSpawnAttr = std::ptr::null_mut();
        let mut actions: PosixSpawnFileActions = std::ptr::null_mut();
        let mut pid: i32 = 0;
        // SAFETY: 所有指针在本作用域内有效；attr / actions 用完即销毁。
        let code = unsafe {
            if posix_spawnattr_init(&mut attr) != 0 {
                bail!("posix_spawnattr_init 失败");
            }
            if posix_spawn_file_actions_init(&mut actions) != 0 {
                posix_spawnattr_destroy(&mut attr);
                bail!("posix_spawn_file_actions_init 失败");
            }
            let mut code = posix_spawnattr_setflags(&mut attr, POSIX_SPAWN_SETPGROUP);
            if code == 0 {
                code = responsibility_spawnattrs_setdisclaim(&mut attr, 1);
            }
            if code == 0 {
                code = posix_spawn_file_actions_adddup2(&mut actions, stdin_read.as_raw_fd(), STDIN);
            }
            if code == 0 {
                code = posix_spawn_file_actions_adddup2(&mut actions, stdout_write.as_raw_fd(), STDOUT);
            }
            if code == 0 {
                code = posix_spawn(
                    &mut pid,
                    path.as_ptr(),
                    &actions,
                    &attr,
                    argv.as_ptr(),
                    envp.as_ptr(),
                );
            }
            posix_spawn_file_actions_destroy(&mut actions);
            posix_spawnattr_destroy(&mut attr);
            code
        };
        if code != 0 {
            bail!(
                "无法启动 {}：{}",
                program.display(),
                std::io::Error::from_raw_os_error(code)
            );
        }
        // 子进程那一端在父进程里必须关掉，否则对端永远读不到 EOF。
        drop(stdin_read);
        drop(stdout_write);
        Ok(DisclaimedChild {
            stdin: Some(Box::new(stdin_write)),
            stdout: Some(Box::new(stdout_read)),
            inner: Inner::Spawned { pid, reaped: false },
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn pipes_round_trip_and_env_is_applied() {
        let mut child = spawn(
            Path::new("/bin/sh"),
            &[OsStr::new("-c"), OsStr::new("read line; echo \"$line:$CODEX_PLUS_PROBE\"")],
            &[("CODEX_PLUS_PROBE".to_string(), "ok".to_string())],
        )
        .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"hello\n").unwrap();
        drop(stdin);
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line.trim(), "hello:ok");
        child.wait();
    }

    #[test]
    fn clean_spawn_passes_only_the_given_env() {
        // 当前进程一定有 HOME；clean 模式下子进程不该看到它。
        assert!(std::env::var_os("HOME").is_some());
        let mut child = spawn_clean(
            Path::new("/bin/sh"),
            &[OsStr::new("-c"), OsStr::new("echo \"${HOME:-none}:$CODEX_PLUS_PROBE\"")],
            &[("CODEX_PLUS_PROBE".to_string(), "ok".to_string())],
        )
        .unwrap();
        drop(child.stdin.take());
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line.trim(), "none:ok");
        child.wait();
    }

    #[test]
    fn missing_program_is_an_error_not_a_panic() {
        assert!(spawn(Path::new("/nonexistent/codex-plus-probe"), &[], &[]).is_err());
    }
}
