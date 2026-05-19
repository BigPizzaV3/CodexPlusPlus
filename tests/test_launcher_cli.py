from pathlib import Path

import pytest

from codex_session_delete import cli, launcher
from codex_session_delete.launcher import build_codex_command, launch_codex_app, packaged_app_user_model_id


class FakeServer:
    port = 57321

    def __init__(self):
        self.shutdown_called = False
        self.server_close_called = False

    def shutdown(self):
        self.shutdown_called = True

    def server_close(self):
        self.server_close_called = True


class FakeProcess:
    def __init__(self):
        self.waited = False

    def wait(self):
        self.waited = True


def test_launch_codex_windows_adds_remote_debugging_port(monkeypatch):
    app_dir = Path("C:/Codex/app")
    popen_calls = []
    monkeypatch.setattr(launcher.subprocess, "Popen", lambda args, **kw: popen_calls.append(args))

    launch_codex_app(app_dir, 9229)

    assert popen_calls
    assert str(app_dir / "Codex.exe") in popen_calls[0][0] or str(app_dir / "codex.exe") in popen_calls[0][0]
    assert "--remote-debugging-port=9229" in popen_calls[0]


def test_launch_codex_windows_allows_devtools_websocket_origin(monkeypatch):
    app_dir = Path("C:/Codex/app")
    popen_calls = []
    monkeypatch.setattr(launcher.subprocess, "Popen", lambda args, **kw: popen_calls.append(args))

    launch_codex_app(app_dir, 9229)

    assert "--remote-allow-origins=http://127.0.0.1:9229" in popen_calls[0]


def test_launch_codex_injects_detected_local_proxy(monkeypatch):
    app_dir = Path("C:/Codex/app")
    popen_calls = []
    monkeypatch.delenv("HTTP_PROXY", raising=False)
    monkeypatch.delenv("HTTPS_PROXY", raising=False)
    monkeypatch.delenv("ALL_PROXY", raising=False)
    monkeypatch.delenv("http_proxy", raising=False)
    monkeypatch.delenv("https_proxy", raising=False)
    monkeypatch.delenv("all_proxy", raising=False)
    monkeypatch.setattr(launcher, "local_proxy_url", lambda: "http://127.0.0.1:7897")
    monkeypatch.setattr(launcher.subprocess, "Popen", lambda args, **kw: popen_calls.append((args, kw)))

    launch_codex_app(app_dir, 9229)

    assert popen_calls[0][1]["env"]["HTTP_PROXY"] == "http://127.0.0.1:7897"
    assert popen_calls[0][1]["env"]["HTTPS_PROXY"] == "http://127.0.0.1:7897"


def test_launch_codex_keeps_explicit_proxy(monkeypatch):
    monkeypatch.setenv("HTTPS_PROXY", "http://127.0.0.1:9999")
    monkeypatch.setattr(launcher, "local_proxy_url", lambda: (_ for _ in ()).throw(AssertionError("should not auto-detect")))

    env = launcher.codex_process_environment()

    assert env["HTTPS_PROXY"] == "http://127.0.0.1:9999"


def test_launch_codex_macos_uses_open_command(monkeypatch, tmp_path):
    app = tmp_path / "Codex.app"
    (app / "Contents" / "MacOS").mkdir(parents=True)
    run_calls = []
    monkeypatch.setattr(launcher.subprocess, "run", lambda args, **kw: run_calls.append(args))

    proc = launch_codex_app(app, 9229)

    assert proc is None
    assert len(run_calls) == 1
    assert run_calls[0][0] == "open"
    assert "-a" in run_calls[0]
    assert str(app) in run_calls[0]


def test_packaged_app_user_model_id_from_windowsapps_path():
    app_dir = Path("C:/Program Files/WindowsApps/OpenAI.Codex_26.506.2212.0_x64__2p2nqsd0c76g0/app")

    assert packaged_app_user_model_id(app_dir) == "OpenAI.Codex_2p2nqsd0c76g0!App"


def test_packaged_app_user_model_id_ignores_non_packaged_path():
    app_dir = Path("C:/Codex/app")

    assert packaged_app_user_model_id(app_dir) is None


def test_launch_uses_packaged_activation_for_windowsapps(monkeypatch):
    app_dir = Path("C:/Program Files/WindowsApps/OpenAI.Codex_26.506.2212.0_x64__2p2nqsd0c76g0/app")
    activated = []
    launched = []
    monkeypatch.setattr(launcher.sys, "platform", "win32")
    monkeypatch.setattr(
        launcher,
        "activate_packaged_app",
        lambda aumid, arguments: activated.append((aumid, arguments)) or 1234,
    )
    monkeypatch.setattr(launcher.subprocess, "Popen", lambda command: launched.append(command))

    assert launcher.launch_codex_app(app_dir, 9229) == 1234

    assert activated == [(
        "OpenAI.Codex_2p2nqsd0c76g0!App",
        "--remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229",
    )]
    assert launched == []


def test_windows_port_selector_uses_ephemeral_port_when_default_is_busy(monkeypatch):
    monkeypatch.setattr(launcher.sys, "platform", "win32")
    monkeypatch.setattr(launcher, "_can_bind_loopback_port", lambda port: port != 9229)
    monkeypatch.setattr(launcher, "_find_available_loopback_port", lambda: 43001)

    assert launcher.select_windows_loopback_port(9229) == 43001


def test_non_windows_port_selector_keeps_requested_port(monkeypatch):
    monkeypatch.setattr(launcher.sys, "platform", "darwin")
    monkeypatch.setattr(launcher, "_can_bind_loopback_port", lambda port: False)

    assert launcher.select_windows_loopback_port(9229) == 9229


def test_cli_keeps_helper_server_alive_after_injection(monkeypatch):
    waited = []
    monkeypatch.setattr(cli, "launch_and_inject", lambda *args: (FakeServer(), None))
    monkeypatch.setattr(cli, "wait_for_shutdown", lambda server, proc, debug_port=9229: waited.append((server.port, debug_port)))

    exit_code = cli.main([])

    assert exit_code == 0
    assert waited == [(57321, 9229)]


def test_cli_launch_subcommand_keeps_helper_server_alive_after_injection(monkeypatch):
    waited = []
    calls = []
    monkeypatch.setattr(cli, "launch_and_inject", lambda *args: calls.append(args) or (FakeServer(), None))
    monkeypatch.setattr(cli, "wait_for_shutdown", lambda server, proc, debug_port=9229: waited.append((server.port, debug_port)))

    exit_code = cli.main(["launch"])

    assert exit_code == 0
    assert waited == [(57321, 9229)]
    assert len(calls) == 1


def test_cli_default_db_path_uses_codex_home(monkeypatch, tmp_path):
    monkeypatch.setenv("CODEX_HOME", str(tmp_path / "profile-home"))

    args = cli.build_parser().parse_args(["launch"])

    assert args.db == tmp_path / "profile-home" / "state_5.sqlite"


def test_cli_install_dispatches_to_platform_installer(monkeypatch, tmp_path):
    calls = []
    monkeypatch.setattr(cli, "install_codex_plus_plus", lambda options: calls.append(options))

    exit_code = cli.main(["install", "--install-root", str(tmp_path), "--launcher-command", "python -m codex_session_delete"])

    assert exit_code == 0
    assert len(calls) == 1
    assert calls[0].install_root == tmp_path
    assert calls[0].launcher_command == "python -m codex_session_delete"


def test_cli_uninstall_dispatches_to_platform_installer(monkeypatch, tmp_path):
    calls = []
    monkeypatch.setattr(cli, "uninstall_codex_plus_plus", lambda options: calls.append(options))

    exit_code = cli.main(["uninstall", "--install-root", str(tmp_path), "--remove-data"])

    assert exit_code == 0
    assert len(calls) == 1
    assert calls[0].install_root == tmp_path
    assert calls[0].remove_data is True


def test_launch_retries_injection_until_codex_page_is_ready(monkeypatch, tmp_path):
    attempts = []
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: FakeServer())
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: None)

    def inject_after_retry(*args, **kwargs):
        attempts.append(args)
        if len(attempts) == 1:
            raise RuntimeError("CDP page not ready")
        return launcher.cdp.InjectionResult(websocket_url="ws://page", bridge_socket=None, result={"result": {}})

    monkeypatch.setattr(launcher, "inject_file_into_all_pages", inject_after_retry)
    monkeypatch.setattr(launcher, "evaluate_user_scripts", lambda websocket_url, script: None)
    monkeypatch.setattr(launcher.time, "sleep", lambda seconds: None)

    server, proc = launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert server.port == 57321
    assert len(attempts) == 2


def test_launch_and_inject_attaches_existing_healthy_helper(monkeypatch, tmp_path):
    started = []
    injected = []
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "helper_health_ok", lambda port, host="127.0.0.1": port == 57321)
    monkeypatch.setattr(launcher, "_log_runtime_event", lambda message: None)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: started.append((args, kwargs)) or FakeServer())
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: None)
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: injected.append(args) or {"result": {}})

    server, proc = launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert isinstance(server, launcher.AttachedHelperServer)
    assert server.port == 57321
    assert proc is None
    assert started == []
    assert injected[0][2] == 57321


def test_helper_health_ok_checks_helper_endpoint(monkeypatch):
    class Response:
        def raise_for_status(self):
            return None

        def json(self):
            return {"ok": True}

    class Session:
        trust_env = True

        def get(self, url, timeout):
            assert url == "http://127.0.0.1:57321/health"
            assert timeout == 1
            assert self.trust_env is False
            return Response()

    monkeypatch.setattr(launcher.requests, "Session", lambda: Session())

    assert launcher.helper_health_ok(57321) is True


def test_check_and_reinject_bridge_reinjects_when_bridge_missing(monkeypatch, tmp_path):
    events = []
    runtime = launcher.CodexPlusRuntime("ws://page", type("Scripts", (), {"build_enabled_bundle": lambda self: ""})(), 9229)
    monkeypatch.setattr(launcher, "evaluate_script", lambda websocket_url, script: {"result": {"result": {"value": False}}})
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: events.append(("inject", args, kwargs)))
    monkeypatch.setattr(launcher, "_log_runtime_event", lambda message: events.append(("log", message)))

    assert launcher.check_and_reinject_bridge(9229, tmp_path / "renderer.js", 57321, object(), object(), runtime) is True

    assert any(event[0] == "inject" for event in events)
    assert any(event[0] == "log" and "renderer bridge missing" in event[1] for event in events)


def test_inject_with_retry_tracks_all_injected_page_targets(monkeypatch, tmp_path):
    evaluations = []
    runtime = launcher.CodexPlusRuntime(
        None,
        type("Scripts", (), {"build_enabled_bundle": lambda self: "window.__userScript = true;"})(),
        9229,
    )
    injections = {
        "main": launcher.cdp.InjectionResult(websocket_url="ws://main", bridge_socket=None, result={"result": "main"}),
        "child": launcher.cdp.InjectionResult(websocket_url="ws://child", bridge_socket=None, result={"result": "child"}),
    }

    def inject_pages(*args, **kwargs):
        for injection in injections.values():
            kwargs["on_injection"](injection)
        return launcher.cdp.MultiPageInjection(injections=injections)

    monkeypatch.setattr(launcher, "inject_file_into_all_pages", inject_pages)
    monkeypatch.setattr(launcher, "evaluate_user_scripts", lambda websocket_url, script: evaluations.append((websocket_url, script)))

    result = launcher.inject_with_retry(9229, tmp_path / "renderer.js", 57321, object(), object(), runtime)

    assert result.websocket_url == "ws://main"
    assert runtime.websocket_urls == {"ws://main", "ws://child"}
    assert evaluations == [
        ("ws://main", "window.__userScript = true;"),
        ("ws://child", "window.__userScript = true;"),
    ]


def test_reload_user_scripts_drops_closed_page_targets(monkeypatch):
    evaluations = []
    runtime = launcher.CodexPlusRuntime(
        "ws://closed",
        type(
            "Scripts",
            (),
            {
                "build_enabled_bundle": lambda self: "window.__userScript = true;",
                "inventory": lambda self: {"global_enabled": True},
            },
        )(),
        9229,
    )
    runtime.websocket_urls.update({"ws://closed", "ws://open"})

    def evaluate(websocket_url, script):
        evaluations.append((websocket_url, script))
        if websocket_url == "ws://closed":
            raise RuntimeError("target closed")

    monkeypatch.setattr(launcher, "evaluate_user_scripts", evaluate)

    result = runtime.reload_user_scripts()

    assert result["target_count"] == 1
    assert runtime.websocket_urls == {"ws://open"}
    assert runtime.websocket_url == "ws://open"
    assert ("ws://open", "window.__userScript = true;") in evaluations


def test_launch_and_inject_returns_windows_packaged_process_id(monkeypatch, tmp_path):
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: FakeServer())
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: 1234)
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: {"result": {}})

    server, proc = launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert server.port == 57321
    assert proc == 1234


def test_shutdown_helper_leaves_attached_helper_running():
    server = launcher.AttachedHelperServer(57321)

    launcher.shutdown_helper(server)

    assert server.port == 57321


def test_launch_and_inject_runs_provider_sync_before_launch_when_enabled(monkeypatch, tmp_path):
    events = []
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: FakeServer())
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: {"result": {}})
    monkeypatch.setattr(launcher, "backend_settings", lambda: type("Settings", (), {"provider_sync_enabled": True})())
    monkeypatch.setattr(launcher, "run_provider_sync", lambda: events.append("sync") or type("Result", (), {"status": "synced", "message": "ok"})())
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: events.append("launch") or 1234)

    launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert events == ["sync", "launch"]


def test_launch_and_inject_skips_provider_sync_when_disabled(monkeypatch, tmp_path):
    events = []
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: FakeServer())
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: {"result": {}})
    monkeypatch.setattr(launcher, "backend_settings", lambda: type("Settings", (), {"provider_sync_enabled": False})())
    monkeypatch.setattr(launcher, "run_provider_sync", lambda: (_ for _ in ()).throw(AssertionError("sync should not run")))
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: events.append("launch") or 1234)

    launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert events == ["launch"]


def test_launch_and_inject_closes_helper_when_injection_fails(monkeypatch, tmp_path):
    server = FakeServer()
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "helper_health_ok", lambda port, host="127.0.0.1": False)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: server)
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: 1234)
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: (_ for _ in ()).throw(RuntimeError("inject failed")))

    with pytest.raises(RuntimeError, match="inject failed"):
        launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert server.shutdown_called is True
    assert server.server_close_called is True


def test_launch_uses_resolved_app_dir(monkeypatch, tmp_path):
    launched = []
    mac_app = tmp_path / "Applications" / "OpenAI Codex.app"
    executable = mac_app / "Contents" / "MacOS" / "Codex"
    executable.parent.mkdir(parents=True)
    executable.write_text("#!/bin/sh\n", encoding="utf-8")
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: mac_app)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: FakeServer())
    monkeypatch.setattr(launcher.subprocess, "run", lambda args, **kw: launched.append(args))
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: {"result": {}})

    launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert str(executable) not in launched[0]
    assert "open" in launched[0]


def test_cli_stops_existing_windows_launchers_before_launch(monkeypatch):
    commands = []
    monkeypatch.setattr(cli.sys, "platform", "win32")
    monkeypatch.setattr(cli.os, "getpid", lambda: 9876)
    monkeypatch.setattr(cli.subprocess, "run", lambda command, **kwargs: commands.append((command, kwargs)))

    cli.stop_existing_windows_launchers()

    assert len(commands) == 1
    command, kwargs = commands[0]
    assert command[:3] == ["powershell", "-NoProfile", "-Command"]
    assert "codex_session_delete" in command[3]
    assert "pythonw?" in command[3]
    assert "Stop-Process" in command[3]
    assert kwargs["env"]["CODEX_PLUS_PLUS_PID"] == "9876"
    assert kwargs["check"] is False


def test_cli_skips_launcher_cleanup_on_non_windows(monkeypatch):
    commands = []
    monkeypatch.setattr(cli.sys, "platform", "linux")
    monkeypatch.setattr(cli.subprocess, "run", lambda command, **kwargs: commands.append((command, kwargs)))

    cli.stop_existing_windows_launchers()

    assert commands == []


def test_cli_launch_runs_launcher_cleanup_before_injection(monkeypatch):
    events = []
    monkeypatch.setattr(cli, "stop_existing_windows_launchers", lambda: events.append("cleanup"))
    monkeypatch.setattr(cli, "launch_and_inject", lambda *args: events.append("launch") or (FakeServer(), None))
    monkeypatch.setattr(cli, "wait_for_shutdown", lambda server, proc, debug_port=9229: events.append("wait"))

    exit_code = cli.main(["launch"])

    assert exit_code == 0
    assert events == ["cleanup", "launch", "wait"]


def test_cli_launch_checks_update_before_injection(monkeypatch):
    events = []
    monkeypatch.setattr(cli, "stop_existing_windows_launchers", lambda: events.append("cleanup"))
    monkeypatch.setattr(cli, "maybe_print_update_notice", lambda: events.append("check-update"))
    monkeypatch.setattr(cli, "launch_and_inject", lambda *args: events.append("launch") or (FakeServer(), None))
    monkeypatch.setattr(cli, "wait_for_shutdown", lambda server, proc, debug_port=9229: events.append("wait"))

    exit_code = cli.main(["launch"])

    assert exit_code == 0
    assert events == ["cleanup", "check-update", "launch", "wait"]


def test_cli_update_notice_ignores_network_errors(monkeypatch, capsys):
    monkeypatch.setattr(cli.updater, "check_for_update", lambda: (_ for _ in ()).throw(RuntimeError("offline")))

    cli.maybe_print_update_notice()

    assert capsys.readouterr().out == ""


def test_cli_setup_alias_installs_with_default_launcher(monkeypatch):
    calls = []
    monkeypatch.setattr(cli, "install_codex_plus_plus", lambda options: calls.append(options))

    exit_code = cli.main(["setup"])

    assert exit_code == 0
    assert len(calls) == 1
    assert calls[0].install_root is None
    assert calls[0].launcher_command is None


def test_cli_check_update_prints_latest_release(monkeypatch, capsys):
    class Release:
        version = "v1.0.5"
        url = "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.0.5"
        body = "fixes"

    monkeypatch.setattr(cli.updater, "is_source_tree_mode", lambda: False)
    monkeypatch.setattr(cli.updater, "check_for_update", lambda: Release())

    exit_code = cli.main(["check-update"])

    assert exit_code == 0
    output = capsys.readouterr().out
    assert "发现新版本 v1.0.5" in output
    assert "CodexPlusPlus/releases/tag/v1.0.5" in output


def test_cli_check_update_reports_current_version(monkeypatch, capsys):
    monkeypatch.setattr(cli.updater, "check_for_update", lambda: None)
    monkeypatch.setattr(cli.updater, "is_source_tree_mode", lambda: False)

    exit_code = cli.main(["check-update"])

    assert exit_code == 0
    assert "当前已是最新版本" in capsys.readouterr().out


def test_cli_check_update_reports_source_tree_migration_mode(monkeypatch, capsys):
    monkeypatch.setattr(cli.updater, "is_source_tree_mode", lambda: True)
    monkeypatch.setattr(cli.updater, "check_for_update", lambda: (_ for _ in ()).throw(AssertionError("should not check release version")))

    exit_code = cli.main(["check-update"])

    assert exit_code == 0
    output = capsys.readouterr().out
    assert "源码目录运行" in output
    assert "update" in output


def test_cli_update_migrates_source_tree_to_release_install(monkeypatch, capsys):
    class Release:
        version = "v1.0.5"
        url = "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.0.5"
        body = "fixes"
        asset_name = "CodexPlusPlus.zip"

    calls = []
    monkeypatch.setattr(cli.updater, "is_source_tree_mode", lambda: True)
    monkeypatch.setattr(cli.updater, "fetch_latest_release", lambda: Release())
    monkeypatch.setattr(cli.updater, "perform_update", lambda release: calls.append(release) or object())

    exit_code = cli.main(["update"])

    assert exit_code == 0
    assert calls[0].version == "v1.0.5"
    output = capsys.readouterr().out
    assert "源码目录运行" in output
    assert "迁移到 Release 安装" in output
    assert "更新完成" in output


def test_cli_update_installs_latest_release(monkeypatch, tmp_path, capsys):
    class Release:
        version = "v1.0.5"
        url = "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.0.5"
        body = "fixes"

    calls = []
    monkeypatch.setattr(cli.updater, "is_source_tree_mode", lambda: False)
    monkeypatch.setattr(cli.updater, "check_for_update", lambda: Release())
    monkeypatch.setattr(cli.updater, "perform_update", lambda release: calls.append(release) or object())

    exit_code = cli.main(["update"])

    assert exit_code == 0
    assert calls[0].version == "v1.0.5"
    assert "更新完成" in capsys.readouterr().out


def test_cli_remove_alias_uninstalls_with_default_options(monkeypatch):
    calls = []
    monkeypatch.setattr(cli, "uninstall_codex_plus_plus", lambda options: calls.append(options))

    exit_code = cli.main(["remove"])

    assert exit_code == 0
    assert len(calls) == 1
    assert calls[0].install_root is None
    assert calls[0].remove_data is False


def test_cli_logs_launch_failure_for_hidden_pythonw(monkeypatch, tmp_path):
    log_path = tmp_path / "codex-plus.log"
    monkeypatch.setattr(cli, "launch_and_inject", lambda *args: (_ for _ in ()).throw(RuntimeError("inject failed")))
    monkeypatch.setattr(cli, "launch_log_path", lambda: log_path)

    with pytest.raises(RuntimeError, match="inject failed"):
        cli.main(["launch"])

    assert "inject failed" in log_path.read_text(encoding="utf-8")


def test_wait_for_shutdown_waits_for_windows_process_id(monkeypatch):
    server = FakeServer()
    waited = []
    monkeypatch.setattr(cli.sys, "platform", "win32")
    monkeypatch.setattr(cli, "wait_for_windows_process_id", lambda process_id: waited.append(process_id))

    cli.wait_for_shutdown(server, 1234)

    assert waited == [1234]
    assert server.shutdown_called is True
    assert server.server_close_called is True


def test_wait_for_shutdown_waits_for_popen_like_process():
    server = FakeServer()
    proc = FakeProcess()

    cli.wait_for_shutdown(server, proc)

    assert proc.waited is True
    assert server.shutdown_called is True
    assert server.server_close_called is True


def test_is_macos_codex_running_falls_back_to_ps(monkeypatch):
    monkeypatch.setattr(cli, "is_codex_cdp_page_available", lambda debug_port=9229: False)
    monkeypatch.setattr(cli, "is_macos_codex_process_running", lambda: True)

    assert cli.is_macos_codex_running() is True


def test_is_codex_cdp_page_available_returns_true_for_codex_page(monkeypatch):
    monkeypatch.setattr(cli, "list_targets", lambda debug_port: [{"type": "page", "title": "Codex", "webSocketDebuggerUrl": "ws://page"}])

    assert cli.is_codex_cdp_page_available() is True
