from __future__ import annotations

import json
import os
import re
import shutil
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any


PROVIDER_ID_RE = re.compile(r"^[A-Za-z0-9_-]+$")


@dataclass(frozen=True)
class AuthSnapshot:
    path: Path
    status: str
    auth_mode: str
    has_chatgpt_tokens: bool
    has_openai_api_key: bool
    message: str = ""

    def to_dict(self) -> dict[str, object]:
        return {
            "status": self.status,
            "path": str(self.path),
            "auth_mode": self.auth_mode,
            "has_chatgpt_tokens": self.has_chatgpt_tokens,
            "has_openai_api_key": self.has_openai_api_key,
            "message": self.message,
        }


@dataclass(frozen=True)
class ProviderSummary:
    path: Path
    status: str
    model: str
    model_provider: str
    provider_name: str
    base_url: str
    has_bearer_token: bool
    requires_openai_auth: bool
    message: str = ""

    def to_dict(self) -> dict[str, object]:
        return {
            "status": self.status,
            "path": str(self.path),
            "model": self.model,
            "model_provider": self.model_provider,
            "provider_name": self.provider_name,
            "base_url": self.base_url,
            "has_bearer_token": self.has_bearer_token,
            "requires_openai_auth": self.requires_openai_auth,
            "message": self.message,
        }


@dataclass(frozen=True)
class ProviderSpec:
    provider_id: str
    base_url: str
    bearer_token: str
    model: str = ""
    display_name: str = ""


def default_codex_home() -> Path:
    return Path.home() / ".codex"


def _string(value: object) -> str:
    return str(value).strip() if isinstance(value, str) else ""


def _auth_path(home: Path) -> Path:
    return home / "auth.json"


def _config_path(home: Path) -> Path:
    return home / "config.toml"


def _load_json(path: Path) -> dict[str, object]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"{path} is not a JSON object")
    return data


def _has_tokens(value: object) -> bool:
    return isinstance(value, dict) and bool(value)


def _atomic_write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = path.with_name(f"{path.name}.tmp")
    temp_path.write_text(text, encoding="utf-8")
    os.replace(temp_path, path)


def inspect_auth(path: Path) -> AuthSnapshot:
    try:
        payload = _load_json(path)
    except Exception as exc:
        return AuthSnapshot(path, "failed", "", False, False, str(exc))
    auth_mode = _string(payload.get("auth_mode"))
    has_tokens = _has_tokens(payload.get("tokens"))
    has_key = bool(_string(payload.get("OPENAI_API_KEY")))
    if has_tokens and auth_mode == "chatgpt" and not has_key:
        return AuthSnapshot(path, "ok", auth_mode, True, False, "ChatGPT identity is preserved")
    if has_tokens:
        return AuthSnapshot(path, "repairable", auth_mode, True, has_key, "ChatGPT tokens exist but auth mode needs normalization")
    return AuthSnapshot(path, "not_chatgpt", auth_mode, False, has_key, "Run codex login before enabling Mobile provider guard")


def normalize_chatgpt_auth(path: Path) -> AuthSnapshot:
    payload = _load_json(path)
    if not _has_tokens(payload.get("tokens")):
        raise ValueError("ChatGPT tokens are missing; run codex login first")
    payload["auth_mode"] = "chatgpt"
    payload["OPENAI_API_KEY"] = None
    _atomic_write(path, json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
    return inspect_auth(path)


def _load_toml(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    return tomllib.loads(path.read_text(encoding="utf-8"))


def _effective_config(config: dict[str, Any]) -> dict[str, Any]:
    effective = dict(config)
    profile = _string(config.get("profile"))
    profiles = config.get("profiles")
    if profile and isinstance(profiles, dict) and isinstance(profiles.get(profile), dict):
        effective.update(profiles[profile])
    return effective


def _provider_config(config: dict[str, Any], provider_id: str) -> tuple[str, dict[str, Any] | None]:
    providers = config.get("model_providers")
    if not isinstance(providers, dict):
        return provider_id, None
    if provider_id and isinstance(providers.get(provider_id), dict):
        return provider_id, providers[provider_id]
    provider_items = [(name, provider) for name, provider in providers.items() if isinstance(name, str) and isinstance(provider, dict)]
    if not provider_id and len(provider_items) == 1:
        return provider_items[0]
    return provider_id, None


def _provider_token(provider: dict[str, Any]) -> str:
    for key in ("experimental_bearer_token", "api_key", "apikey", "bearer_token", "token"):
        value = _string(provider.get(key))
        if value:
            return value
    return ""


def read_provider_summary(path: Path) -> ProviderSummary:
    try:
        config = _load_toml(path)
    except Exception as exc:
        return ProviderSummary(path, "failed", "", "", "", "", False, False, str(exc))
    effective = _effective_config(config)
    model = _string(effective.get("model"))
    configured_provider = _string(effective.get("model_provider"))
    provider_id, provider = _provider_config(config, configured_provider)
    if not isinstance(provider, dict):
        return ProviderSummary(path, "not_configured", model, provider_id, "", "", False, False, "Active provider is not configured")
    provider_name = _string(provider.get("name")) or provider_id
    base_url = _string(provider.get("base_url"))
    has_token = bool(_provider_token(provider))
    requires_openai_auth = bool(provider.get("requires_openai_auth", False))
    status = "ok" if base_url and has_token and requires_openai_auth else "incomplete"
    return ProviderSummary(path, status, model, provider_id, provider_name, base_url, has_token, requires_openai_auth)


def _provider_spec(payload: dict[str, object]) -> ProviderSpec:
    provider_id = _string(payload.get("provider_id") or payload.get("provider") or payload.get("name"))
    base_url = _string(payload.get("base_url")).rstrip("/")
    bearer_token = _string(payload.get("bearer_token") or payload.get("experimental_bearer_token") or payload.get("token"))
    if not provider_id or not PROVIDER_ID_RE.match(provider_id):
        raise ValueError("Provider id must contain only letters, numbers, underscores, or hyphens")
    if not base_url.startswith(("http://", "https://")):
        raise ValueError("Provider base_url must start with http:// or https://")
    if not bearer_token:
        raise ValueError("Provider bearer token is required")
    return ProviderSpec(provider_id, base_url, bearer_token, _string(payload.get("model")), _string(payload.get("display_name") or payload.get("provider_name")))


def _set_top_level_string(text: str, key: str, value: str) -> str:
    lines = text.splitlines()
    pattern = re.compile(rf"^(\s*{re.escape(key)}\s*=\s*).*$")
    for index, line in enumerate(lines):
        if line.lstrip().startswith("["):
            break
        if pattern.match(line):
            lines[index] = f'{key} = {json.dumps(value)}'
            return "\n".join(lines).rstrip() + "\n"
    lines.insert(0, f'{key} = {json.dumps(value)}')
    return "\n".join(lines).rstrip() + "\n"


def _remove_provider_table(text: str, provider_id: str) -> str:
    pattern = re.compile(rf"(?ms)^\[model_providers\.{re.escape(provider_id)}\]\n.*?(?=^\[|\Z)")
    return pattern.sub("", text).rstrip() + "\n"


def write_provider_config(path: Path, spec: ProviderSpec) -> ProviderSummary:
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    text = _set_top_level_string(text, "model_provider", spec.provider_id)
    if spec.model:
        text = _set_top_level_string(text, "model", spec.model)
    text = _remove_provider_table(text, spec.provider_id)
    table = "\n".join(
        [
            "",
            f"[model_providers.{spec.provider_id}]",
            f"name = {json.dumps(spec.display_name or spec.provider_id)}",
            f"base_url = {json.dumps(spec.base_url)}",
            f"experimental_bearer_token = {json.dumps(spec.bearer_token)}",
            "requires_openai_auth = true",
            "",
        ]
    )
    _atomic_write(path, text.rstrip() + "\n" + table)
    return read_provider_summary(path)


def _backup_dir(home: Path) -> Path:
    root = home / "backups_state" / "mobile-provider-guard"
    backup = root / time.strftime("%Y%m%d%H%M%S")
    suffix = 0
    while backup.exists():
        suffix += 1
        backup = root / f"{time.strftime('%Y%m%d%H%M%S')}-{suffix}"
    backup.mkdir(parents=True)
    return backup


def _backup_files(home: Path) -> Path:
    backup = _backup_dir(home)
    for name in ("auth.json", "config.toml"):
        source = home / name
        if source.exists():
            shutil.copy2(source, backup / name)
    return backup


def _restore_backup(home: Path, backup: Path) -> None:
    for name in ("auth.json", "config.toml"):
        source = backup / name
        if source.exists():
            shutil.copy2(source, home / name)


def mobile_provider_status(codex_home: Path | None = None) -> dict[str, object]:
    home = codex_home or default_codex_home()
    auth = inspect_auth(_auth_path(home))
    provider = read_provider_summary(_config_path(home))
    status = "ok" if auth.status == "ok" and provider.status == "ok" else "needs_attention"
    return {"status": status, "codex_home": str(home), "auth": auth.to_dict(), "provider": provider.to_dict()}


def apply_mobile_provider_guard(payload: dict[str, object], codex_home: Path | None = None) -> dict[str, object]:
    home = codex_home or default_codex_home()
    try:
        spec = _provider_spec(payload)
        auth = inspect_auth(_auth_path(home))
        if not auth.has_chatgpt_tokens:
            return {"status": "failed", "message": auth.message, "auth": auth.to_dict()}
        backup = _backup_files(home)
        try:
            next_auth = normalize_chatgpt_auth(_auth_path(home))
            provider = write_provider_config(_config_path(home), spec)
        except Exception:
            _restore_backup(home, backup)
            raise
        return {
            "status": "ok",
            "message": "ChatGPT identity preserved; third-party provider activated",
            "backup_dir": str(backup),
            "auth": next_auth.to_dict(),
            "provider": provider.to_dict(),
        }
    except Exception as exc:
        status = mobile_provider_status(home)
        status["status"] = "failed"
        status["message"] = str(exc)
        return status
