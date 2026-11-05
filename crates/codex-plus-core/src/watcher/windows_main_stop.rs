use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::windows_integration::{StopProcessEnvironment, StopProcessOwner};

#[derive(Clone, PartialEq, Eq)]
struct ProcessFacts {
    birth: u64,
    image: PathBuf,
    arguments: Vec<String>,
    owner: StopProcessOwner,
    environment: StopProcessEnvironment,
}

struct StopContext {
    executable: String,
    user_data: Option<String>,
    owner: StopProcessOwner,
    app_data: String,
    owned_debug_port: Option<u16>,
}

trait HeldMainProcess {
    fn id(&self) -> u32;
    fn alive(&self) -> Result<bool>;
    fn possible_target(&self, context: &StopContext) -> Result<bool>;
    fn facts(&self) -> Result<ProcessFacts>;
    fn request_close(&self) -> Result<bool>;
    fn terminate(&self) -> Result<()>;
}

/// 停止已经核验的 Windows Codex 主进程，并等待原句柄确认退出。
///
/// 仅处理调用管理器的默认主上下文：双方须属于同一用户、登录及 Windows
/// 会话，APPDATA 相同，且均没有已知的 userData/agent 隔离覆盖。调用者须已确认使用
/// 官方默认启动约定；这不是对所有未来版本实际 userData 计算方式的证明。
/// 已就绪后台会话的端口只用于拒绝不属于默认上下文的候选，不用于扩大停止范围。
/// 不使用调试端口兜底，不遍历或结束子进程，不重新按 PID 终止进程。
#[cfg(windows)]
pub fn stop_windows_main_codex_processes_and_wait(
    expected_executable: &Path,
    owned_debug_port: Option<u16>,
) -> Result<usize> {
    use crate::windows_integration::{VerifiedStopProcess, bounded_process_snapshot};

    let expected_name = expected_executable
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| crate::app_paths::is_windows_desktop_app_executable_name(name))
        .ok_or_else(|| anyhow::anyhow!("缺少可信的 Codex 桌面应用入口"))?;
    let executable = normalized_path(expected_executable)?;
    let manager = VerifiedStopProcess::capture(std::process::id(), false)?
        .ok_or_else(|| anyhow::anyhow!("无法持有当前管理器的进程身份"))?;
    let environment = manager.stop_environment()?;
    anyhow::ensure!(
        environment.user_data.is_none() && environment.agent_run_id.is_none(),
        "管理器存在隔离环境覆盖，无法按已知官方默认启动约定识别主实例"
    );
    let app_data = normalized_path_text(
        environment
            .app_data
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("无法核实管理器 APPDATA"))?,
    )?;
    let context = StopContext {
        executable,
        user_data: None,
        owner: manager.stop_owner()?,
        app_data,
        owned_debug_port,
    };
    let candidates = || -> Result<Vec<u32>> {
        Ok(bounded_process_snapshot(65_536)?
            .into_iter()
            .filter(|process| process.exe_file.eq_ignore_ascii_case(expected_name))
            .map(|process| process.process_id)
            .collect())
    };
    let stopped = capture_and_stop(
        candidates()?,
        VerifiedStopProcess::capture,
        &context,
        Duration::from_millis(super::RESTART_STOP_WAIT_TIMEOUT_MS),
        || std::thread::sleep(Duration::from_millis(super::RESTART_STOP_WAIT_INTERVAL_MS)),
    )?;
    ensure_no_live_targets(candidates()?, VerifiedStopProcess::capture, &context)?;
    Ok(stopped)
}

#[cfg(windows)]
impl HeldMainProcess for crate::windows_integration::VerifiedStopProcess {
    fn id(&self) -> u32 {
        self.process_id
    }

    fn alive(&self) -> Result<bool> {
        self.is_alive()
    }

    fn possible_target(&self, context: &StopContext) -> Result<bool> {
        let image = normalized_path(&self.image()?)?;
        if !image_matches(&context.executable, &image) {
            return Ok(false);
        }
        Ok(!parsed_arguments(&self.arguments()?, &image)?.child)
    }

    fn facts(&self) -> Result<ProcessFacts> {
        Ok(ProcessFacts {
            birth: self.birth_id,
            image: self.image()?,
            arguments: self.arguments()?,
            owner: self.stop_owner()?,
            environment: self.stop_environment()?,
        })
    }

    fn terminate(&self) -> Result<()> {
        crate::windows_integration::VerifiedStopProcess::terminate(self)
    }

    fn request_close(&self) -> Result<bool> {
        self.request_main_close()
    }
}

fn capture_and_stop<P: HeldMainProcess>(
    ids: impl IntoIterator<Item = u32>,
    mut capture: impl FnMut(u32, bool) -> Result<Option<P>>,
    context: &StopContext,
    timeout: Duration,
    mut wait: impl FnMut(),
) -> Result<usize> {
    let mut held = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // 所有候选均持有句柄、完成核验后才允许首次终止；后续候选失败不会造成部分预先停止。
    for id in ids {
        anyhow::ensure!(id != 0 && seen.insert(id), "主实例快照包含无效或重复 PID");
        let Some(process) =
            capture(id, false).with_context(|| format!("无法核验 Codex 候选 PID {id}"))?
        else {
            continue;
        };
        let Some(facts) = candidate_facts(&process, context)? else {
            continue;
        };
        if !target_matches(&facts, context)? {
            if process.alive()? {
                reject_skipped_owned_session(&facts, context)?;
            }
            continue;
        }
        let Some(stoppable) = capture(id, true)
            .with_context(|| format!("无法取得 Codex 主进程 PID {id} 的停止权限"))?
        else {
            anyhow::ensure!(!process.alive()?, "无法取得仍存活的 Codex 主进程句柄");
            continue;
        };
        if !process.alive()? {
            continue;
        }
        let current = stoppable
            .facts()
            .context("无法核验持有停止权限的 Codex 句柄")?;
        anyhow::ensure!(
            stoppable.alive()? && current == facts && target_matches(&current, context)?,
            "获取停止权限时 Codex PID {id} 的实例身份发生变化"
        );
        held.push((stoppable, current));
    }
    revalidate_held(&held, context)?;
    let mut closing = Vec::new();
    for (process, _) in &held {
        if process.request_close()? {
            closing.push(process);
        }
    }
    if !closing.is_empty() {
        let grace_deadline = Instant::now() + timeout.min(Duration::from_millis(1_500));
        loop {
            let mut pending = false;
            for process in &closing {
                pending |= process.alive()?;
            }
            if !pending || Instant::now() >= grace_deadline {
                break;
            }
            wait();
        }
    }
    // WM_CLOSE 是正常退出机会，不是退出证明；强制前再次核验所有仍存活的原句柄。
    revalidate_held(&held, context)?;
    let mut failure = None;
    for (process, _) in &held {
        if let Err(error) = process.terminate() {
            failure.get_or_insert_with(|| {
                error.context(format!("停止 Codex PID {} 失败", process.id()))
            });
        }
    }
    let deadline = Instant::now() + timeout;
    loop {
        let mut alive = Vec::new();
        for (process, _) in &held {
            match process.alive() {
                Ok(true) => alive.push(process.id()),
                Ok(false) => {}
                Err(error) => {
                    failure.get_or_insert_with(|| error.context("无法确认原 Codex 句柄退出"));
                }
            }
        }
        if alive.is_empty() {
            return match failure {
                Some(error) => Err(error),
                None => Ok(held.len()),
            };
        }
        if Instant::now() >= deadline {
            if let Some(error) = failure {
                return Err(error).with_context(|| {
                    format!("Codex 原主进程在等待结束后仍存活（PID {alive:?}）；未继续启动")
                });
            }
            anyhow::bail!("Codex 原主进程在等待结束后仍存活（PID {alive:?}）；未继续启动");
        }
        wait();
    }
}

fn candidate_facts<P: HeldMainProcess>(
    process: &P,
    context: &StopContext,
) -> Result<Option<ProcessFacts>> {
    if !process.alive()? {
        return Ok(None);
    }
    match process.possible_target(context) {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(_) if !process.alive()? => return Ok(None),
        Err(error) => return Err(error).context("无法筛选 Codex 主实例"),
    }
    match process.facts() {
        Ok(facts) => Ok(Some(facts)),
        Err(_) if !process.alive()? => Ok(None),
        Err(error) => Err(error).context("无法完整核验 Codex 主实例"),
    }
}

fn reject_skipped_owned_session(facts: &ProcessFacts, context: &StopContext) -> Result<()> {
    if facts.owner == context.owner
        && context.owned_debug_port.is_some()
        && parsed_arguments(&facts.arguments, &normalized_path(&facts.image)?)?.debug_port
            == context.owned_debug_port
    {
        anyhow::bail!("当前后台会话的 Codex 进程不属于可核验的默认用户数据上下文，无法确认停止");
    }
    Ok(())
}

fn ensure_no_live_targets<P: HeldMainProcess>(
    ids: impl IntoIterator<Item = u32>,
    mut capture: impl FnMut(u32, bool) -> Result<Option<P>>,
    context: &StopContext,
) -> Result<()> {
    for id in ids {
        let Some(process) =
            capture(id, false).with_context(|| format!("无法复核 Codex 候选 PID {id}"))?
        else {
            continue;
        };
        let Some(facts) = candidate_facts(&process, context)? else {
            continue;
        };
        if !process.alive()? {
            continue;
        }
        if target_matches(&facts, context)? {
            anyhow::bail!("停止后检测到新的或仍存活的 Codex 主进程 PID {id}，未确认退出");
        }
        reject_skipped_owned_session(&facts, context)?;
    }
    Ok(())
}

fn revalidate_held<P: HeldMainProcess>(
    held: &[(P, ProcessFacts)],
    context: &StopContext,
) -> Result<()> {
    for (process, previous) in held {
        if !process.alive()? {
            continue;
        }
        let current = match process.facts() {
            Ok(facts) => facts,
            Err(_) if !process.alive()? => continue,
            Err(error) => return Err(error).context("停止前无法再次核验 Codex 主实例"),
        };
        anyhow::ensure!(
            current == *previous && target_matches(&current, context)?,
            "停止前 Codex PID {} 的实例身份发生变化",
            process.id()
        );
    }
    Ok(())
}

fn target_matches(facts: &ProcessFacts, context: &StopContext) -> Result<bool> {
    anyhow::ensure!(facts.birth != 0, "缺少 Codex 进程创建身份");
    let image = normalized_path(&facts.image)?;
    if !image_matches(&context.executable, &image) || facts.owner != context.owner {
        return Ok(false);
    }
    let arguments = parsed_arguments(&facts.arguments, &image)?;
    if arguments.child {
        return Ok(false);
    }
    let environment = &facts.environment;
    let app_data = environment
        .app_data
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("无法核实目标 Codex APPDATA"))?;
    if normalized_path_text(app_data)? != context.app_data {
        return Ok(false);
    }
    let environment_dir = environment
        .user_data
        .as_deref()
        .map(normalized_path_text)
        .transpose()?;
    if let (Some(argument), Some(environment)) = (&arguments.user_data, &environment_dir) {
        anyhow::ensure!(
            argument == environment,
            "Codex 命令行与环境中的用户数据目录冲突"
        );
    }
    let effective = arguments.user_data.or(environment_dir);
    match &context.user_data {
        Some(expected) => {
            if effective.as_ref() != Some(expected) {
                return Ok(false);
            }
            anyhow::ensure!(
                environment.agent_run_id.is_none(),
                "Codex 含 agent 隔离覆盖，无法安全证明用户数据上下文"
            );
            Ok(true)
        }
        None => Ok(effective.is_none() && environment.agent_run_id.is_none()),
    }
}

struct ParsedArguments {
    child: bool,
    user_data: Option<String>,
    debug_port: Option<u16>,
}

fn parsed_arguments(arguments: &[String], image: &str) -> Result<ParsedArguments> {
    let first = arguments
        .first()
        .ok_or_else(|| anyhow::anyhow!("Codex 命令行缺少 argv[0]"))?;
    anyhow::ensure!(
        normalized_path_text(first)? == image,
        "Codex argv[0] 与实际映像不一致"
    );
    let mut child = None;
    let mut user_data = None;
    let mut debug_port = None;
    let mut index = 1;
    while index < arguments.len() {
        let argument = &arguments[index];
        anyhow::ensure!(
            argument.trim() != "--",
            "无法安全识别含参数终止标记的 Codex 命令行"
        );
        let key = super::windows_codex_switch_name(argument);
        if key.is_some_and(|key| {
            key.eq_ignore_ascii_case("type")
                || key.eq_ignore_ascii_case("user-data-dir")
                || key.eq_ignore_ascii_case("remote-debugging-port")
        }) {
            let key = key.unwrap();
            let inline = argument.trim().split_once('=').map(|(_, value)| value);
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    arguments
                        .get(index)
                        .filter(|value| super::windows_codex_switch_name(value).is_none())
                        .map(String::as_str)
                        .ok_or_else(|| anyhow::anyhow!("Codex 身份参数缺少值"))?
                }
            };
            anyhow::ensure!(!value.is_empty(), "Codex 身份参数为空");
            if key.eq_ignore_ascii_case("type") {
                anyhow::ensure!(
                    child.replace(value.to_owned()).is_none(),
                    "Codex --type 参数重复"
                );
            } else if key.eq_ignore_ascii_case("user-data-dir") {
                anyhow::ensure!(
                    user_data.replace(normalized_path_text(value)?).is_none(),
                    "Codex --user-data-dir 参数重复"
                );
            } else {
                let port = value
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| anyhow::anyhow!("Codex 调试端口参数无效"))?;
                anyhow::ensure!(debug_port.replace(port).is_none(), "Codex 调试端口参数重复");
            }
        }
        index += 1;
    }
    Ok(ParsedArguments {
        child: child.is_some(),
        user_data,
        debug_port,
    })
}

fn normalized_path(path: &Path) -> Result<String> {
    normalized_path_text(
        path.to_str()
            .ok_or_else(|| anyhow::anyhow!("停止核验路径不是有效 Unicode"))?,
    )
}

fn normalized_path_text(path: &str) -> Result<String> {
    let mut path = path.replace('/', "\\").to_lowercase();
    if let Some(rest) = path.strip_prefix("\\\\?\\unc\\") {
        path = format!("\\\\{rest}");
    } else if let Some(rest) = path.strip_prefix("\\\\?\\") {
        path = rest.to_owned();
    }
    let drive_rooted = path.len() >= 3
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1..3] == *b":\\";
    let body = if drive_rooted {
        &path[3..]
    } else if let Some(body) = path.strip_prefix("\\\\") {
        anyhow::ensure!(
            body.split('\\').filter(|part| !part.is_empty()).count() >= 2,
            "停止核验需要完整 UNC 路径"
        );
        body
    } else {
        anyhow::bail!("停止核验需要绝对 Windows 路径")
    };
    anyhow::ensure!(
        !body.contains(['\0', ':', '"', '*', '?'])
            && body
                .split('\\')
                .all(|part| { part != "." && part != ".." && !part.ends_with([' ', '.']) }),
        "停止核验路径包含无法安全归一化的部分"
    );
    while path.ends_with('\\') && path.len() > 3 {
        path.pop();
    }
    Ok(path)
}

fn image_matches(expected: &str, actual: &str) -> bool {
    if expected == actual {
        return true;
    }
    let Some((expected_family, expected_entry)) = packaged_entry(expected) else {
        return false;
    };
    let Some((actual_family, actual_entry)) = packaged_entry(actual) else {
        return false;
    };
    expected_family == actual_family && expected_entry == actual_entry
}

fn packaged_entry(path: &str) -> Option<((String, String, String), &str)> {
    let (root, after) = path.split_once("\\windowsapps\\")?;
    let (package, entry) = after.split_once('\\')?;
    let segments = package.split('_').collect::<Vec<_>>();
    if segments.len() != 5
        || segments[0].is_empty()
        || segments[4].is_empty()
        || !matches!(segments[2], "x64" | "x86" | "arm64" | "neutral")
        || segments[1].split('.').count() != 4
        || segments[1]
            .split('.')
            .any(|part| part.parse::<u32>().is_err())
        || !entry.starts_with("app\\")
        || entry[4..].contains('\\')
    {
        return None;
    }
    Some((
        (
            root.to_owned(),
            segments[0].to_owned(),
            segments[4].to_owned(),
        ),
        entry,
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    const IMAGE: &str =
        r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3.4_x64__publisher\app\ChatGPT.exe";
    const APP_DATA: &str = r"C:\Users\test\AppData\Roaming";

    fn context(user_data: Option<&str>) -> StopContext {
        StopContext {
            executable: normalized_path_text(IMAGE).unwrap(),
            user_data: user_data.map(|path| normalized_path_text(path).unwrap()),
            owner: StopProcessOwner {
                user_sid: vec![1, 2, 3],
                authentication_id: (10, 0),
                session_id: 1,
            },
            app_data: normalized_path_text(APP_DATA).unwrap(),
            owned_debug_port: None,
        }
    }

    fn facts(arguments: &[&str]) -> ProcessFacts {
        ProcessFacts {
            birth: 100,
            image: PathBuf::from(IMAGE),
            arguments: std::iter::once(IMAGE)
                .chain(arguments.iter().copied())
                .map(str::to_owned)
                .collect(),
            owner: context(None).owner,
            environment: StopProcessEnvironment {
                app_data: Some(APP_DATA.to_owned()),
                user_data: None,
                agent_run_id: None,
            },
        }
    }

    #[derive(Clone)]
    struct FakeProcess(Rc<FakeState>);

    struct FakeState {
        id: u32,
        facts: Vec<ProcessFacts>,
        reads: Cell<usize>,
        terminated: Cell<bool>,
        stubborn: bool,
        read_failure: bool,
        stop_failure: bool,
        graceful: bool,
        window_present: bool,
        close_requests: Cell<usize>,
        events: Rc<RefCell<Vec<u32>>>,
    }

    impl HeldMainProcess for FakeProcess {
        fn id(&self) -> u32 {
            self.0.id
        }

        fn alive(&self) -> Result<bool> {
            Ok(!self.0.terminated.get() || self.0.stubborn)
        }

        fn possible_target(&self, context: &StopContext) -> Result<bool> {
            let facts = &self.0.facts[0];
            let image = normalized_path(&facts.image)?;
            if !image_matches(&context.executable, &image) {
                return Ok(false);
            }
            Ok(!parsed_arguments(&facts.arguments, &image)?.child)
        }

        fn facts(&self) -> Result<ProcessFacts> {
            anyhow::ensure!(!self.0.read_failure, "模拟环境读取失败");
            let reads = self.0.reads.get();
            self.0.reads.set(reads + 1);
            Ok(self.0.facts[reads.min(self.0.facts.len() - 1)].clone())
        }

        fn terminate(&self) -> Result<()> {
            if self.0.terminated.get() && !self.0.stubborn {
                return Ok(());
            }
            self.0.events.borrow_mut().push(self.id());
            anyhow::ensure!(!self.0.stop_failure, "模拟终止失败");
            self.0.terminated.set(true);
            Ok(())
        }

        fn request_close(&self) -> Result<bool> {
            self.0.close_requests.set(self.0.close_requests.get() + 1);
            if self.0.graceful {
                self.0.terminated.set(true);
            }
            Ok(self.0.graceful || self.0.window_present)
        }
    }

    fn fake(id: u32, facts: Vec<ProcessFacts>, events: &Rc<RefCell<Vec<u32>>>) -> FakeProcess {
        FakeProcess(Rc::new(FakeState {
            id,
            facts,
            reads: Cell::new(0),
            terminated: Cell::new(false),
            stubborn: false,
            read_failure: false,
            stop_failure: false,
            graceful: false,
            window_present: false,
            close_requests: Cell::new(0),
            events: events.clone(),
        }))
    }

    fn stop(processes: Vec<FakeProcess>, context: &StopContext) -> Result<usize> {
        capture_and_stop(
            processes
                .iter()
                .map(HeldMainProcess::id)
                .collect::<Vec<_>>(),
            |id, _| Ok(processes.iter().find(|process| process.id() == id).cloned()),
            context,
            Duration::ZERO,
            || panic!("零超时测试不应等待"),
        )
    }

    #[test]
    fn stops_only_verified_main_handles() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let main = fake(1, vec![facts(&["--remote-debugging-port=52106"])], &events);
        let mut child = fake(2, vec![facts(&["--type=renderer"])], &events);
        Rc::get_mut(&mut child.0).unwrap().read_failure = true;
        assert_eq!(stop(vec![main.clone(), child], &context(None)).unwrap(), 1);
        assert_eq!(*events.borrow(), vec![1]);
        assert_eq!(main.0.reads.get(), 4);
    }

    #[test]
    fn graceful_window_close_avoids_forced_termination() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut main = fake(1, vec![facts(&[])], &events);
        Rc::get_mut(&mut main.0).unwrap().graceful = true;
        assert_eq!(stop(vec![main.clone()], &context(None)).unwrap(), 1);
        assert_eq!(main.0.close_requests.get(), 1);
        assert_eq!(main.0.reads.get(), 3);
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn identity_change_after_close_prevents_force() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let initial = facts(&[]);
        let mut changed = initial.clone();
        changed.environment.user_data = Some(r"C:\isolated".to_owned());
        let main = fake(
            1,
            vec![initial.clone(), initial.clone(), initial, changed],
            &events,
        );
        assert!(stop(vec![main.clone()], &context(None)).is_err());
        assert_eq!(main.0.close_requests.get(), 1);
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn window_that_does_not_exit_reaches_bounded_force_after_revalidation() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut main = fake(1, vec![facts(&[])], &events);
        Rc::get_mut(&mut main.0).unwrap().window_present = true;
        assert_eq!(stop(vec![main.clone()], &context(None)).unwrap(), 1);
        assert_eq!(main.0.close_requests.get(), 1);
        assert_eq!(main.0.reads.get(), 4);
        assert_eq!(*events.borrow(), vec![1]);
    }

    #[test]
    fn default_context_preserves_argv_environment_and_agent_isolation() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let argument = facts(&["--user-data-dir", r"C:\test\isolated"]);
        let mut environment = facts(&[]);
        environment.environment.user_data = Some(r"C:\test\isolated".to_owned());
        let mut agent = facts(&[]);
        agent.environment.agent_run_id = Some("isolated-agent".to_owned());
        let processes = [argument, environment, agent]
            .into_iter()
            .enumerate()
            .map(|(index, facts)| fake(index as u32 + 1, vec![facts], &events))
            .collect();
        assert_eq!(stop(processes, &context(None)).unwrap(), 0);
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn windows_switch_variants_preserve_isolated_and_child_processes() {
        for argument in [
            "--USER-DATA-DIR=C:\\isolated",
            "-user-data-dir=C:\\isolated",
            "/User-Data-Dir=C:\\isolated",
            "  --user-data-dir=C:\\isolated  ",
        ] {
            assert!(!target_matches(&facts(&[argument]), &context(None)).unwrap());
        }
        for argument in ["--TYPE=renderer", "-type=renderer", "/Type=renderer"] {
            assert!(!target_matches(&facts(&[argument]), &context(None)).unwrap());
        }
        assert!(
            target_matches(
                &facts(&["--user-data-dir=C:\\one", "/USER-DATA-DIR=C:\\one"]),
                &context(None)
            )
            .is_err()
        );
    }

    #[test]
    fn skipped_owned_session_prevents_stopping_other_targets() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let main = fake(1, vec![facts(&[])], &events);
        let isolated = fake(
            2,
            vec![facts(&[
                "/REMOTE-DEBUGGING-PORT=9329",
                "--user-data-dir=C:\\isolated",
            ])],
            &events,
        );
        let mut expected = context(None);
        expected.owned_debug_port = Some(9329);
        assert!(stop(vec![main, isolated], &expected).is_err());
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn post_stop_snapshot_rejects_a_new_matching_main_process() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let replacement = fake(3, vec![facts(&[])], &events);
        let result =
            ensure_no_live_targets([3], |_, _| Ok(Some(replacement.clone())), &context(None));
        assert!(result.is_err());
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn explicit_context_requires_a_proven_exact_directory() {
        let expected = context(Some(r"C:\test\isolated"));
        let mut environment = facts(&[]);
        environment.environment.user_data = Some(r"C:\test\isolated".to_owned());
        assert!(target_matches(&environment, &expected).unwrap());
        assert!(
            target_matches(&facts(&["--user-data-dir=C:\\test\\isolated"]), &expected).unwrap()
        );
        assert!(!target_matches(&facts(&[]), &expected).unwrap());
        assert!(!target_matches(&facts(&["--user-data-dir=C:\\test\\other"]), &expected).unwrap());
    }

    #[test]
    fn same_basename_in_another_image_or_package_is_preserved() {
        let expected = context(None);
        let mut candidate = facts(&[]);
        candidate.image = PathBuf::from(r"C:\other\ChatGPT.exe");
        assert!(!target_matches(&candidate, &expected).unwrap());
        candidate.image = PathBuf::from(IMAGE.replace("OpenAI.Codex_", "OpenAI.ChatGPT_"));
        assert!(!target_matches(&candidate, &expected).unwrap());
        candidate.image = PathBuf::from(IMAGE.replace("__publisher", "__other"));
        assert!(!target_matches(&candidate, &expected).unwrap());
    }

    #[test]
    fn package_update_retains_the_same_family_and_entry_requirement() {
        let current = normalized_path_text(IMAGE).unwrap();
        let old = normalized_path_text(&IMAGE.replace("1.2.3.4", "1.2.2.0")).unwrap();
        assert!(image_matches(&current, &old));
        assert!(!image_matches(
            &current,
            &old.replace("\\app\\", "\\resources\\")
        ));
        assert!(!image_matches(
            &current,
            &old.replace("chatgpt.exe", "codex.exe")
        ));
        assert!(!image_matches(
            &current,
            &old.replace("c:\\program files\\", "c:\\other\\")
        ));
    }

    #[test]
    fn owner_and_appdata_mismatch_are_preserved_but_missing_appdata_is_an_error() {
        let expected = context(None);
        let mut candidate = facts(&[]);
        candidate.owner.authentication_id.0 += 1;
        assert!(!target_matches(&candidate, &expected).unwrap());
        candidate.owner = expected.owner.clone();
        candidate.owner.session_id += 1;
        assert!(!target_matches(&candidate, &expected).unwrap());
        candidate.owner = expected.owner.clone();
        candidate.owner.user_sid.push(4);
        assert!(!target_matches(&candidate, &expected).unwrap());
        candidate.owner = expected.owner.clone();
        candidate.environment.app_data = Some(r"C:\other\AppData".to_owned());
        assert!(!target_matches(&candidate, &expected).unwrap());
        candidate.environment.app_data = None;
        assert!(target_matches(&candidate, &expected).is_err());
    }

    #[test]
    fn ambiguous_or_malformed_identity_arguments_fail_closed() {
        let cases = [
            vec!["--user-data-dir"],
            vec!["--user-data-dir="],
            vec!["--user-data-dir=relative"],
            vec!["--user-data-dir=C:\\one", "--user-data-dir=C:\\one"],
            vec!["--type=renderer", "--type=renderer"],
            vec!["--remote-debugging-port=no"],
            vec![
                "--remote-debugging-port=52106",
                "--remote-debugging-port=52106",
            ],
            vec!["--"],
        ];
        for arguments in cases {
            assert!(
                target_matches(&facts(&arguments), &context(None)).is_err(),
                "{arguments:?}"
            );
        }
        let mut conflict = facts(&["--user-data-dir=C:\\one"]);
        conflict.environment.user_data = Some(r"C:\two".to_owned());
        assert!(target_matches(&conflict, &context(None)).is_err());
    }

    #[test]
    fn paths_with_spaces_use_the_existing_windows_argv_boundaries() {
        let candidate = facts(&["--user-data-dir", r"C:\a path\an instance"]);
        assert!(target_matches(&candidate, &context(Some(r"C:\a path\an instance"))).unwrap());
        assert_eq!(
            normalized_path_text(r"\\?\C:\A Path\Instance\").unwrap(),
            r"c:\a path\instance"
        );
        for path in [r"C:relative", r"C:\a\..\b", r"C:\a.", r"C:\a ", r"\\server"] {
            assert!(normalized_path_text(path).is_err(), "{path}");
        }
    }

    #[test]
    fn later_capture_or_validation_failure_prevents_every_termination() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let main = fake(1, vec![facts(&[])], &events);
        let bad = fake(2, vec![facts(&["--user-data-dir="])], &events);
        assert!(stop(vec![main.clone(), bad], &context(None)).is_err());
        assert!(events.borrow().is_empty());
        let result = capture_and_stop(
            vec![1, 2],
            |id, _| {
                if id == 1 {
                    Ok(Some(main.clone()))
                } else {
                    anyhow::bail!("模拟 ACCESS_DENIED")
                }
            },
            &context(None),
            Duration::ZERO,
            || {},
        );
        assert!(result.is_err());
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn only_verified_main_candidates_request_stop_rights() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let requests = RefCell::new(Vec::new());
        let main = fake(1, vec![facts(&[])], &events);
        let child = fake(2, vec![facts(&["--type=renderer"])], &events);
        let processes = [main, child];
        let result = capture_and_stop(
            [1, 2],
            |id, terminate| {
                requests.borrow_mut().push((id, terminate));
                Ok(processes.iter().find(|process| process.id() == id).cloned())
            },
            &context(None),
            Duration::ZERO,
            || {},
        );
        assert_eq!(result.unwrap(), 1);
        assert_eq!(*requests.borrow(), vec![(1, false), (1, true), (2, false)]);
        assert_eq!(*events.borrow(), vec![1]);
    }

    #[test]
    fn changed_identity_when_opening_stop_handle_prevents_all_actions() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let initial = fake(1, vec![facts(&[])], &events);
        let mut replacement_facts = facts(&[]);
        replacement_facts.birth += 1;
        let replacement = fake(1, vec![replacement_facts], &events);
        let result = capture_and_stop(
            [1],
            |_, terminate| {
                Ok(Some(if terminate {
                    replacement.clone()
                } else {
                    initial.clone()
                }))
            },
            &context(None),
            Duration::ZERO,
            || {},
        );
        assert!(result.is_err());
        assert_eq!(initial.0.close_requests.get(), 0);
        assert_eq!(replacement.0.close_requests.get(), 0);
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn unreadable_or_changed_identity_prevents_every_termination() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let main = fake(1, vec![facts(&[])], &events);
        let mut unknown = fake(2, vec![facts(&[])], &events);
        Rc::get_mut(&mut unknown.0).unwrap().read_failure = true;
        assert!(stop(vec![main.clone(), unknown], &context(None)).is_err());
        let initial = facts(&[]);
        let mut changed = initial.clone();
        changed.environment.user_data = Some(r"C:\isolated".to_owned());
        let changing = fake(3, vec![initial, changed], &events);
        assert!(stop(vec![main, changing], &context(None)).is_err());
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn zero_creation_identity_prevents_termination() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut candidate = facts(&[]);
        candidate.birth = 0;
        assert!(stop(vec![fake(1, vec![candidate], &events)], &context(None)).is_err());
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn termination_failure_and_surviving_original_handle_are_reported() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut failure = fake(1, vec![facts(&[])], &events);
        Rc::get_mut(&mut failure.0).unwrap().stop_failure = true;
        assert!(stop(vec![failure], &context(None)).is_err());
        let mut stubborn = fake(2, vec![facts(&[])], &events);
        Rc::get_mut(&mut stubborn.0).unwrap().stubborn = true;
        assert!(stop(vec![stubborn], &context(None)).is_err());
        assert_eq!(*events.borrow(), vec![1, 2]);
    }

    #[cfg(windows)]
    struct OwnedTestChild(std::process::Child);

    #[cfg(windows)]
    impl Drop for OwnedTestChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[cfg(windows)]
    #[test]
    fn owned_process_stop_preserves_an_isolated_sibling() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        use crate::windows_integration::{CREATE_NO_WINDOW, VerifiedStopProcess};

        let fixture = tempfile::tempdir().unwrap();
        let app_data = fixture.path().join("appdata");
        let user_data = fixture.path().join("isolated");
        let executable = std::env::current_exe().unwrap();
        let spawn = |isolated: bool| {
            let mut command = Command::new(&executable);
            command
                .args([
                    "--exact",
                    "windows_integration::tests::stop_identity_child_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("CODEX_PLUS_STOP_IDENTITY_TEST_CHILD", "owned-helper")
                .env("APPDATA", &app_data)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW);
            if isolated {
                command.env("CODEX_ELECTRON_USER_DATA_PATH", &user_data);
            }
            OwnedTestChild(command.spawn().unwrap())
        };
        let mut main = spawn(false);
        let mut isolated = spawn(true);
        assert!(main.0.try_wait().unwrap().is_none());
        assert!(isolated.0.try_wait().unwrap().is_none());
        let owner = VerifiedStopProcess::capture(std::process::id(), false)
            .unwrap()
            .unwrap()
            .stop_owner()
            .unwrap();
        let context = StopContext {
            executable: normalized_path(&executable).unwrap(),
            user_data: None,
            owner,
            app_data: normalized_path(&app_data).unwrap(),
            owned_debug_port: None,
        };
        let count = capture_and_stop(
            [main.0.id(), isolated.0.id()],
            VerifiedStopProcess::capture,
            &context,
            Duration::from_secs(5),
            || std::thread::sleep(Duration::from_millis(10)),
        )
        .unwrap();
        assert_eq!(count, 1);
        assert!(main.0.try_wait().unwrap().is_some());
        assert!(isolated.0.try_wait().unwrap().is_none());
        drop(isolated.0.stdin.take());
    }

    #[cfg(windows)]
    #[test]
    fn public_stop_api_stops_only_its_owned_default_main() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        let fixture = tempfile::tempdir().unwrap();
        let app_data = fixture.path().join("appdata");
        std::fs::create_dir(&app_data).unwrap();
        // 原名 driver 不会进入 ChatGPT.exe 快照；仅子进程持有临时 APPDATA。
        let mut driver = OwnedTestChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "watcher::windows_main_stop::tests::public_stop_api_driver",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("CODEX_PLUS_MAIN_STOP_API_TEST_DRIVER", "owned-driver")
                .env("APPDATA", &app_data)
                .stdin(Stdio::null())
                .creation_flags(crate::windows_integration::CREATE_NO_WINDOW)
                .spawn()
                .unwrap(),
        );
        assert!(driver.0.wait().unwrap().success());
        assert_eq!(
            std::fs::read(app_data.join("stop-api-completed")).unwrap(),
            b"ok"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "仅由 public_stop_api_stops_only_its_owned_default_main 启动"]
    fn public_stop_api_driver() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        if std::env::var_os("CODEX_PLUS_MAIN_STOP_API_TEST_DRIVER") != Some("owned-driver".into()) {
            return;
        }
        let original = std::env::current_exe().unwrap();
        assert!(!crate::app_paths::is_windows_desktop_app_executable_name(
            original.file_name().unwrap().to_str().unwrap()
        ));
        let app_data = PathBuf::from(std::env::var_os("APPDATA").unwrap());
        assert!(app_data.is_absolute());
        let fixture = tempfile::tempdir_in(&app_data).unwrap();
        let executable = fixture.path().join("ChatGPT.exe");
        std::fs::copy(&original, &executable).unwrap();
        let spawn = |isolated: bool| {
            let mut command = Command::new(&executable);
            command
                .args([
                    "--exact",
                    "windows_integration::tests::stop_identity_child_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("CODEX_PLUS_STOP_IDENTITY_TEST_CHILD", "owned-helper")
                .env("APPDATA", &app_data)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
            if isolated {
                command.env(
                    "CODEX_ELECTRON_USER_DATA_PATH",
                    fixture.path().join("isolated"),
                );
            }
            OwnedTestChild(command.spawn().unwrap())
        };
        let mut main = spawn(false);
        let mut isolated = spawn(true);
        assert!(main.0.try_wait().unwrap().is_none());
        assert!(isolated.0.try_wait().unwrap().is_none());
        assert_eq!(
            stop_windows_main_codex_processes_and_wait(&executable, None).unwrap(),
            1
        );
        assert!(main.0.try_wait().unwrap().is_some());
        assert!(isolated.0.try_wait().unwrap().is_none());
        assert_eq!(
            stop_windows_main_codex_processes_and_wait(&executable, None).unwrap(),
            0
        );
        assert!(isolated.0.try_wait().unwrap().is_none());
        std::fs::write(app_data.join("stop-api-completed"), b"ok").unwrap();
    }
}
