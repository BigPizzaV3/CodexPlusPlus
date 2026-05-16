import json

from codex_session_delete.mobile_provider_guard import (
    apply_mobile_provider_guard,
    inspect_auth,
    mobile_provider_status,
    normalize_chatgpt_auth,
    read_provider_summary,
)


def write_chatgpt_auth(path, *, mode="apikey", openai_key="sk-old"):
    path.write_text(
        json.dumps(
            {
                "auth_mode": mode,
                "OPENAI_API_KEY": openai_key,
                "tokens": {"access_token": "chatgpt-token", "refresh_token": "refresh-token"},
            }
        ),
        encoding="utf-8",
    )


def test_normalize_chatgpt_auth_preserves_tokens_and_clears_api_key(tmp_path):
    auth = tmp_path / "auth.json"
    write_chatgpt_auth(auth)

    snapshot = normalize_chatgpt_auth(auth)
    payload = json.loads(auth.read_text(encoding="utf-8"))

    assert snapshot.status == "ok"
    assert payload["auth_mode"] == "chatgpt"
    assert payload["OPENAI_API_KEY"] is None
    assert payload["tokens"]["access_token"] == "chatgpt-token"


def test_inspect_codex_auth_rejects_plain_api_key_identity(tmp_path):
    auth = tmp_path / "auth.json"
    auth.write_text(json.dumps({"auth_mode": "apikey", "OPENAI_API_KEY": "sk-only"}), encoding="utf-8")

    snapshot = inspect_auth(auth)

    assert snapshot.status == "not_chatgpt"
    assert snapshot.has_openai_api_key is True
    assert snapshot.has_chatgpt_tokens is False


def test_apply_mobile_provider_guard_keeps_chatgpt_identity_and_writes_provider(tmp_path):
    codex_home = tmp_path / ".codex"
    codex_home.mkdir()
    write_chatgpt_auth(codex_home / "auth.json")
    (codex_home / "config.toml").write_text('model = "gpt-5.5"\nmodel_provider = "openai"\n', encoding="utf-8")

    result = apply_mobile_provider_guard(
        {
            "provider_id": "apiname",
            "display_name": "API Name",
            "base_url": "https://relay.example.com/v1",
            "bearer_token": "relay-token",
            "model": "gpt-5.5",
        },
        codex_home,
    )

    assert result["status"] == "ok"
    payload = json.loads((codex_home / "auth.json").read_text(encoding="utf-8"))
    assert payload["auth_mode"] == "chatgpt"
    assert payload["OPENAI_API_KEY"] is None
    assert payload["tokens"]["access_token"] == "chatgpt-token"
    text = (codex_home / "config.toml").read_text(encoding="utf-8")
    assert 'model_provider = "apiname"' in text
    assert 'experimental_bearer_token = "relay-token"' in text
    assert "requires_openai_auth = true" in text
    assert result["provider"]["has_bearer_token"] is True
    assert result["provider"]["requires_openai_auth"] is True
    assert (codex_home / "backups_state" / "mobile-provider-guard").exists()


def test_apply_mobile_provider_guard_refuses_when_chatgpt_tokens_missing(tmp_path):
    codex_home = tmp_path / ".codex"
    codex_home.mkdir()
    (codex_home / "auth.json").write_text(json.dumps({"auth_mode": "apikey", "OPENAI_API_KEY": "sk-only"}), encoding="utf-8")
    (codex_home / "config.toml").write_text("", encoding="utf-8")

    result = apply_mobile_provider_guard(
        {
            "provider_id": "apiname",
            "base_url": "https://relay.example.com/v1",
            "bearer_token": "relay-token",
        },
        codex_home,
    )

    assert result["status"] == "failed"
    assert result["auth"]["has_chatgpt_tokens"] is False
    assert "codex login" in result["message"]
    assert not (codex_home / "backups_state").exists()


def test_mobile_provider_status_reports_active_provider_without_leaking_token(tmp_path):
    codex_home = tmp_path / ".codex"
    codex_home.mkdir()
    write_chatgpt_auth(codex_home / "auth.json", mode="chatgpt", openai_key=None)
    (codex_home / "config.toml").write_text(
        """
model = "gpt-5.5"
model_provider = "local"

[model_providers.local]
name = "local"
base_url = "http://127.0.0.1:8317/v1"
experimental_bearer_token = "local-secret"
requires_openai_auth = true
""".strip(),
        encoding="utf-8",
    )

    status = mobile_provider_status(codex_home)

    assert status["status"] == "ok"
    assert status["provider"]["model_provider"] == "local"
    assert status["provider"]["has_bearer_token"] is True
    assert "local-secret" not in json.dumps(status)
    assert read_provider_summary(codex_home / "config.toml").status == "ok"
