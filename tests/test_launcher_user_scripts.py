from codex_session_delete.launcher import handle_bridge_request, read_codex_config_model, read_codex_model_catalog
from codex_session_delete.models import ExportResult, ExportStatus
from codex_session_delete.settings_store import SettingsStore
from codex_session_delete.user_scripts import UserScriptManager


class FakeDeleteService:
    def delete(self, session):
        raise AssertionError("delete should not be called")

    def undo(self, undo_token):
        raise AssertionError("undo should not be called")

    def find_archived_thread_by_title(self, title):
        return None


class FakeExportService:
    def export(self, session):
        return ExportResult(ExportStatus.EXPORTED, session.session_id, "Exported", filename="thread.md", markdown="# Thread\n")


class FakeRuntime:
    def __init__(self, manager):
        self.user_scripts = manager
        self.injected = []
        self.devtools_opened = False
        self.repaired = False
        self.ads_payload = {"version": 1, "ads": [{"id": "runtime-ad", "type": "normal", "title": "Runtime Ad", "description": "Loaded", "url": "https://0029.org", "highlights": []}]}

    def reload_user_scripts(self):
        bundle = self.user_scripts.build_enabled_bundle()
        self.injected.append(bundle)
        return self.user_scripts.inventory()

    def open_devtools(self):
        self.devtools_opened = True
        return {"status": "ok"}

    def backend_status(self):
        return {"status": "ok", "message": "后端已连接"}

    def repair_backend(self):
        self.repaired = True
        return {"status": "ok", "message": "后端已修复"}

    def ads(self):
        return self.ads_payload

    def codex_config_model(self):
        return {"status": "ok", "model": "qwen3-coder", "model_provider": "dashscope", "provider_name": "DashScope", "models": ["qwen3-coder"]}

    def codex_model_catalog(self):
        return {
            "status": "ok",
            "model": "qwen3-coder",
            "default_model": "qwen3-coder",
            "model_provider": "dashscope",
            "provider_name": "DashScope",
            "models": ["qwen3-coder", "deepseek-coder"],
            "sources": [{"type": "config", "status": "ok", "models": 2}],
        }


def test_handle_bridge_request_lists_user_scripts(tmp_path):
    builtin = tmp_path / "builtin"
    user = tmp_path / "user"
    builtin.mkdir()
    (builtin / "demo.js").write_text("window.demo = true;", encoding="utf-8")
    manager = UserScriptManager(builtin, user, tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/user-scripts/list", {}, runtime)

    assert result["enabled"] is True
    assert result["scripts"][0]["key"] == "builtin:demo.js"


def test_handle_bridge_request_updates_user_script_toggles(tmp_path):
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    global_result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/user-scripts/set-enabled", {"enabled": False}, runtime)
    script_result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/user-scripts/set-script-enabled", {"key": "user:a.js", "enabled": False}, runtime)

    assert global_result["enabled"] is False
    assert script_result["scripts"] == []
    assert manager.load_config().scripts["user:a.js"] is False


def test_handle_bridge_request_reports_and_repairs_backend_status(tmp_path):
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    status = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/backend/status", {}, runtime)
    repaired = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/backend/repair", {}, runtime)

    assert status == {"status": "ok", "message": "后端已连接"}
    assert runtime.repaired is True
    assert repaired == {"status": "ok", "message": "后端已修复"}

def test_handle_bridge_request_returns_codex_model_catalog(tmp_path):
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/codex-model-catalog", {}, runtime)

    assert result["status"] == "ok"
    assert result["model"] == "qwen3-coder"
    assert result["models"] == ["qwen3-coder", "deepseek-coder"]


def test_handle_bridge_request_keeps_legacy_codex_config_model_route(tmp_path):
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/codex-config-model", {}, runtime)

    assert result["status"] == "ok"
    assert result["models"] == ["qwen3-coder", "deepseek-coder"]


def test_read_codex_config_model_uses_active_profile(tmp_path):
    config = tmp_path / "config.toml"
    config.write_text(
        """
model = "gpt-5.5"
model_provider = "openai"
profile = "china"

[profiles.china]
model = "qwen3-coder"
model_provider = "dashscope"

[model_providers.dashscope]
name = "DashScope"
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
""".strip(),
        encoding="utf-8",
    )

    result = read_codex_config_model(config)

    assert result["status"] == "ok"
    assert result["model"] == "qwen3-coder"
    assert result["model_provider"] == "dashscope"
    assert result["provider_name"] == "DashScope"
    assert result["models"] == ["qwen3-coder"]


def test_read_codex_model_catalog_fetches_models_from_config_provider(tmp_path):
    config = tmp_path / "config.toml"
    auth = tmp_path / "auth.json"
    config.write_text(
        """
model_provider = "mycodex"
model = "qwen3-coder"

[model_providers.mycodex]
name = "My Codex"
base_url = "https://relay.example.com/v1"
""".strip(),
        encoding="utf-8",
    )
    auth.write_text('{"OPENAI_API_KEY":"sk-test"}', encoding="utf-8")
    requests = []

    class Response:
        status_code = 200

        def json(self):
            return {"object": "list", "data": [{"id": "qwen3-coder"}, {"id": "deepseek-coder"}]}

    def fake_get(url, **kwargs):
        requests.append((url, kwargs))
        return Response()

    result = read_codex_model_catalog(config, auth, env={}, requests_get=fake_get)

    assert result["status"] == "ok"
    assert result["default_model"] == "qwen3-coder"
    assert result["models"] == ["qwen3-coder", "deepseek-coder"]
    assert result["sources"][0]["type"] == "config"
    assert result["sources"][0]["auth"] == "present"
    assert requests[0][0] == "https://relay.example.com/v1/models"
    assert requests[0][1]["headers"]["Authorization"] == "Bearer sk-test"


def test_read_codex_model_catalog_reports_unsupported_responses_api(tmp_path):
    config = tmp_path / "config.toml"
    auth = tmp_path / "auth.json"
    config.write_text(
        """
model_provider = "mycodex"
model = "qwen3-coder"

[model_providers.mycodex]
name = "My Codex"
base_url = "https://relay.example.com/v1"
""".strip(),
        encoding="utf-8",
    )
    auth.write_text('{"OPENAI_API_KEY":"sk-test"}', encoding="utf-8")
    posts = []

    class ModelsResponse:
        status_code = 200

        def json(self):
            return {"data": [{"id": "qwen3-coder"}]}

    class UnsupportedResponsesResponse:
        status_code = 404

        def json(self):
            return {"error": {"message": "No route for /v1/responses"}}

    def fake_post(url, **kwargs):
        posts.append((url, kwargs))
        return UnsupportedResponsesResponse()

    result = read_codex_model_catalog(
        config,
        auth,
        env={},
        requests_get=lambda *args, **kwargs: ModelsResponse(),
        requests_post=fake_post,
    )

    assert result["responses_api"]["status"] == "unsupported"
    assert result["responses_api"]["endpoint"] == "https://relay.example.com/v1/responses"
    assert result["sources"][0]["responses_api"]["status"] == "unsupported"
    assert posts[0][0] == "https://relay.example.com/v1/responses"
    assert posts[0][1]["json"]["model"] == "qwen3-coder"


def test_read_codex_model_catalog_reports_incompatible_responses_payload(tmp_path):
    config = tmp_path / "config.toml"
    auth = tmp_path / "auth.json"
    config.write_text(
        """
model_provider = "mycodex"

[model_providers.mycodex]
name = "My Codex"
base_url = "https://relay.example.com/v1"
api_key = "sk-test"
""".strip(),
        encoding="utf-8",
    )

    class ModelsResponse:
        status_code = 200

        def json(self):
            return {"data": [{"id": "qwen3-coder"}]}

    class IncompatibleResponsesResponse:
        status_code = 400

        def json(self):
            return {"error": {"message": "Unrecognized request argument supplied: max_output_tokens"}}

    result = read_codex_model_catalog(
        config,
        auth,
        env={},
        requests_get=lambda *args, **kwargs: ModelsResponse(),
        requests_post=lambda *args, **kwargs: IncompatibleResponsesResponse(),
    )

    assert result["responses_api"]["status"] == "unsupported"
    assert "max_output_tokens" in result["responses_api"]["message"]


def test_read_codex_model_catalog_does_not_misreport_auth_failure_as_unsupported_responses_api(tmp_path):
    config = tmp_path / "config.toml"
    auth = tmp_path / "auth.json"
    config.write_text(
        """
model_provider = "mycodex"

[model_providers.mycodex]
name = "My Codex"
base_url = "https://relay.example.com/v1"
api_key = "bad-key"
""".strip(),
        encoding="utf-8",
    )

    class ModelsResponse:
        status_code = 200

        def json(self):
            return {"data": [{"id": "qwen3-coder"}]}

    class AuthFailedResponse:
        status_code = 401

        def json(self):
            return {"error": {"message": "Invalid API key"}}

    result = read_codex_model_catalog(
        config,
        auth,
        env={},
        requests_get=lambda *args, **kwargs: ModelsResponse(),
        requests_post=lambda *args, **kwargs: AuthFailedResponse(),
    )

    assert result["responses_api"]["status"] == "failed"
    assert "Invalid API key" in result["responses_api"]["message"]


def test_read_codex_model_catalog_merges_environment_and_config_sources(tmp_path):
    config = tmp_path / "config.toml"
    config.write_text(
        """
model_provider = "dashscope"

[model_providers.dashscope]
name = "DashScope"
base_url = "https://dashscope.example.com/compatible-mode/v1"
api_key = "config-key"
""".strip(),
        encoding="utf-8",
    )
    auth = tmp_path / "missing-auth.json"

    class Response:
        def __init__(self, payload):
            self.status_code = 200
            self.payload = payload

        def json(self):
            return self.payload

    def fake_get(url, **kwargs):
        if "env.example.com" in url:
            return Response({"data": [{"id": "moonshot-v1"}, {"id": "qwen3-coder"}]})
        return Response({"data": [{"id": "qwen3-coder"}, {"id": "deepseek-coder"}]})

    result = read_codex_model_catalog(
        config,
        auth,
        env={"OPENAI_BASE_URL": "https://env.example.com/v1", "OPENAI_API_KEY": "env-key"},
        requests_get=fake_get,
    )

    assert result["status"] == "ok"
    assert result["models"] == ["moonshot-v1", "qwen3-coder", "deepseek-coder"]
    assert [source["type"] for source in result["sources"]] == ["environment", "config"]


def test_read_codex_model_catalog_uses_auth_json_for_env_base_url(tmp_path):
    config = tmp_path / "missing-config.toml"
    auth = tmp_path / "auth.json"
    auth.write_text('{"OPENAI_API_KEY":"sk-auth"}', encoding="utf-8")
    requests = []

    class Response:
        status_code = 200

        def json(self):
            return {"data": [{"id": "mimo-v2.5-pro"}]}

    def fake_get(url, **kwargs):
        requests.append((url, kwargs))
        return Response()

    result = read_codex_model_catalog(
        config,
        auth,
        env={"OPENAI_BASE_URL": "https://user:pass@relay.example.com/v1?secret=1"},
        requests_get=fake_get,
    )

    assert result["status"] == "ok"
    assert result["models"] == ["mimo-v2.5-pro"]
    assert requests[0][1]["headers"]["Authorization"] == "Bearer sk-auth"
    assert result["sources"][0]["base_url"] == "https://relay.example.com/v1"
    assert result["sources"][0]["endpoint"] == "https://relay.example.com/v1/models"


def test_read_codex_model_catalog_uses_single_config_provider_without_model_provider(tmp_path):
    config = tmp_path / "config.toml"
    auth = tmp_path / "auth.json"
    config.write_text(
        """
[model_providers.only]
name = "Only Provider"
base_url = "https://only.example.com/v1"
api_key = "config-key"
""".strip(),
        encoding="utf-8",
    )

    class Response:
        status_code = 200

        def json(self):
            return {"data": [{"id": "qwen3-coder"}]}

    result = read_codex_model_catalog(config, auth, env={}, requests_get=lambda *args, **kwargs: Response())

    assert result["status"] == "ok"
    assert result["model_provider"] == "only"
    assert result["provider_name"] == "Only Provider"
    assert result["models"] == ["qwen3-coder"]


def test_handle_bridge_request_gets_backend_settings(monkeypatch, tmp_path):
    store = SettingsStore(tmp_path / "settings.json")
    store.update({"providerSyncEnabled": True})
    monkeypatch.setattr("codex_session_delete.launcher.SettingsStore", lambda: store)
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/settings/get", {}, runtime)

    assert result == {"providerSyncEnabled": True}


def test_handle_bridge_request_sets_backend_settings(monkeypatch, tmp_path):
    store = SettingsStore(tmp_path / "settings.json")
    monkeypatch.setattr("codex_session_delete.launcher.SettingsStore", lambda: store)
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/settings/set", {"providerSyncEnabled": True}, runtime)

    assert result == {"providerSyncEnabled": True}
    assert store.load().provider_sync_enabled is True


def test_handle_bridge_request_returns_ads(tmp_path):
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/ads", {}, runtime)

    assert result["ads"][0]["id"] == "runtime-ad"



def test_handle_bridge_request_exports_markdown(tmp_path):
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    exported = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/export-markdown", {"session_id": "s1", "title": "First"}, runtime)

    assert exported["status"] == "exported"
    assert exported["filename"] == "thread.md"



def test_handle_bridge_request_returns_zed_remote_status(monkeypatch, tmp_path):
    from codex_session_delete import zed_remote

    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)
    monkeypatch.setattr(
        zed_remote,
        "zed_remote_status",
        lambda: {"status": "ok", "platformSupported": True, "zedAppFound": True, "zedCliFound": False},
    )

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/zed-remote/status", {}, runtime)

    assert result == {"status": "ok", "platformSupported": True, "zedAppFound": True, "zedCliFound": False}


def test_handle_bridge_request_resolves_zed_remote_host(monkeypatch, tmp_path):
    from codex_session_delete import zed_remote

    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)
    payload = {"hostId": "remote-ssh-codex-managed:remote"}

    monkeypatch.setattr(
        zed_remote,
        "resolve_ssh_target_response",
        lambda received: {"status": "ok", "ssh": {"user": "longnv", "host": "192.168.100.31", "port": None}, "payload": received},
    )

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/zed-remote/resolve-host", payload, runtime)

    assert result == {
        "status": "ok",
        "ssh": {"user": "longnv", "host": "192.168.100.31", "port": None},
        "payload": payload,
    }


def test_handle_bridge_request_opens_zed_remote(monkeypatch, tmp_path):
    from codex_session_delete import zed_remote

    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)
    opened = []

    def fake_open(payload):
        opened.append(payload)
        return {"status": "ok", "url": "ssh://example.com/home/app.py"}

    monkeypatch.setattr(zed_remote, "open_zed_remote", fake_open)
    payload = {"ssh": {"host": "example.com"}, "path": "/home/app.py"}

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/zed-remote/open", payload, runtime)

    assert result == {"status": "ok", "url": "ssh://example.com/home/app.py"}
    assert opened == [payload]
