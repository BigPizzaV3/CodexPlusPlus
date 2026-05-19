import pytest

from codex_session_delete import zed_remote
from codex_session_delete.zed_remote import SshTarget, ZedRemoteError


def test_build_zed_remote_url_with_user_host_port_and_encoded_path():
    url = zed_remote.build_zed_remote_url(
        SshTarget(user="alice", host="example.com", port=2222),
        "/home/alice/My Project/你好.py",
    )

    assert url == "ssh://alice@example.com:2222/home/alice/My%20Project/%E4%BD%A0%E5%A5%BD.py"


def test_build_zed_remote_url_allows_host_without_user():
    url = zed_remote.build_zed_remote_url(SshTarget(user="", host="box.internal", port=None), "/srv/app/main.py")

    assert url == "ssh://box.internal/srv/app/main.py"


def test_build_zed_remote_url_preserves_path_segment_whitespace():
    url = zed_remote.build_zed_remote_url(
        SshTarget(user="alice", host="example.com", port=None),
        "/tmp/has trailing /file name .py",
    )

    assert url == "ssh://alice@example.com/tmp/has%20trailing%20/file%20name%20.py"


@pytest.mark.parametrize(
    ("target", "path", "message"),
    [
        (SshTarget(user="alice", host="", port=None), "/a.py", "Cannot determine remote SSH host"),
        (SshTarget(user="alice", host="example.com", port=70000), "/a.py", "Invalid SSH port"),
        (SshTarget(user="alice", host="example.com", port=22), "relative.py", "Remote path must be absolute"),
        (SshTarget(user="alice", host="example.com", port=22), "", "Remote path is required"),
    ],
)
def test_build_zed_remote_url_rejects_invalid_inputs(target, path, message):
    with pytest.raises(ZedRemoteError) as exc:
        zed_remote.build_zed_remote_url(target, path)

    assert message in str(exc.value)


def test_target_from_payload_prefers_structured_ssh_fields():
    target = zed_remote.target_from_payload({"ssh": {"user": "deploy", "host": "10.0.0.5", "port": "2200"}})

    assert target == SshTarget(user="deploy", host="10.0.0.5", port=2200)


def test_target_from_payload_splits_codex_managed_authority():
    target = zed_remote.target_from_payload({"ssh": {"host": "longnv@192.168.100.31"}})

    assert target == SshTarget(user="longnv", host="192.168.100.31", port=None)


def test_resolve_ssh_target_from_global_state_for_codex_managed_connection():
    state = {
        "codex-managed-remote-connections": [
            {
                "hostId": "remote-ssh-codex-managed:remote",
                "displayName": "remote",
                "source": "codex-managed",
                "hostname": "longnv@192.168.100.31",
                "sshPort": None,
            }
        ]
    }

    target = zed_remote.resolve_ssh_target_from_global_state(state, "remote-ssh-codex-managed:remote")

    assert target == SshTarget(user="longnv", host="192.168.100.31", port=None)


def test_resolve_ssh_target_response_reports_missing_host_id():
    result = zed_remote.resolve_ssh_target_response({"hostId": ""})

    assert result == {"status": "failed", "message": "Remote host id is required"}


def test_status_reports_supported_platform_and_detection(monkeypatch, tmp_path):
    zed_app = tmp_path / "Zed.app"
    zed_app.mkdir()
    monkeypatch.setattr(zed_remote, "candidate_zed_app_paths", lambda: [zed_app])
    monkeypatch.setattr(zed_remote.shutil, "which", lambda name: "/usr/local/bin/zed" if name == "zed" else None)
    monkeypatch.setattr(zed_remote.sys, "platform", "darwin")

    status = zed_remote.zed_remote_status()

    assert status == {
        "status": "ok",
        "platformSupported": True,
        "zedAppFound": True,
        "zedCliFound": True,
        "zedAppPath": str(zed_app),
        "zedCliPath": "/usr/local/bin/zed",
    }


def test_open_zed_remote_uses_open_app_on_macos_when_app_exists(monkeypatch, tmp_path):
    zed_app = tmp_path / "Zed.app"
    zed_app.mkdir()
    calls = []
    monkeypatch.setattr(zed_remote, "candidate_zed_app_paths", lambda: [zed_app])
    monkeypatch.setattr(zed_remote.shutil, "which", lambda name: None)
    monkeypatch.setattr(zed_remote.sys, "platform", "darwin")
    monkeypatch.setattr(zed_remote.subprocess, "run", lambda command, check: calls.append((command, check)))

    result = zed_remote.open_zed_remote({"ssh": {"user": "alice", "host": "example.com"}, "path": "/home/alice/a.py"})

    assert result["status"] == "ok"
    assert result["url"] == "ssh://alice@example.com/home/alice/a.py"
    assert calls == [(["open", "-a", str(zed_app), "ssh://alice@example.com/home/alice/a.py"], True)]


def test_open_zed_remote_uses_zed_cli_when_app_is_missing(monkeypatch):
    calls = []
    monkeypatch.setattr(zed_remote, "candidate_zed_app_paths", lambda: [])
    monkeypatch.setattr(zed_remote.shutil, "which", lambda name: "/usr/local/bin/zed" if name == "zed" else None)
    monkeypatch.setattr(zed_remote.sys, "platform", "linux")
    monkeypatch.setattr(zed_remote.subprocess, "run", lambda command, check: calls.append((command, check)))

    result = zed_remote.open_zed_remote({"ssh": {"host": "box.internal"}, "path": "/srv/app.py"})

    assert result["status"] == "ok"
    assert calls == [(["/usr/local/bin/zed", "ssh://box.internal/srv/app.py"], True)]


def test_open_zed_remote_returns_failed_response_for_validation_error(monkeypatch):
    monkeypatch.setattr(zed_remote, "candidate_zed_app_paths", lambda: [])
    monkeypatch.setattr(zed_remote.shutil, "which", lambda name: "/usr/local/bin/zed")

    result = zed_remote.open_zed_remote({"ssh": {"host": ""}, "path": "/a.py"})

    assert result == {"status": "failed", "message": "Cannot determine remote SSH host for this file"}


@pytest.mark.parametrize("host", ["bad host", "bad\thost", "bad\nhost", "bad/host", "bad?host", "bad#host", "user@host"])
def test_build_zed_remote_url_rejects_unsafe_authority_hosts(host):
    with pytest.raises(ZedRemoteError) as exc:
        zed_remote.build_zed_remote_url(SshTarget(user="alice", host=host, port=None), "/a.py")

    assert "Invalid SSH host" in str(exc.value)


def test_build_zed_remote_url_allows_bracketed_ipv6_host():
    url = zed_remote.build_zed_remote_url(SshTarget(user="alice", host="[::1]", port=2222), "/home/alice/a.py")

    assert url == "ssh://alice@[::1]:2222/home/alice/a.py"
