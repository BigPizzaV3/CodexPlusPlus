from __future__ import annotations

import base64
from dataclasses import dataclass
from enum import Enum


class DeleteStatus(str, Enum):
    SERVER_DELETED = "server_deleted"
    LOCAL_DELETED = "local_deleted"
    PARTIAL = "partial"
    FAILED = "failed"
    UNDONE = "undone"


class ExportStatus(str, Enum):
    EXPORTED = "exported"
    FAILED = "failed"


@dataclass(frozen=True)
class SessionRef:
    session_id: str
    title: str

    def __post_init__(self) -> None:
        if not self.session_id:
            raise ValueError("session_id is required")


@dataclass(frozen=True)
class DeleteResult:
    status: DeleteStatus
    session_id: str
    message: str
    undo_token: str | None = None
    backup_path: str | None = None

    def to_dict(self) -> dict[str, str | None]:
        return {
            "status": self.status.value,
            "session_id": self.session_id,
            "message": self.message,
            "undo_token": self.undo_token,
            "backup_path": self.backup_path,
        }


@dataclass(frozen=True)
class ExportResult:
    status: ExportStatus
    session_id: str
    message: str
    filename: str | None = None
    markdown: str | None = None

    def to_dict(self) -> dict[str, str | None]:
        return {
            "status": self.status.value,
            "session_id": self.session_id,
            "message": self.message,
            "filename": self.filename,
            "markdown": self.markdown,
        }


@dataclass(frozen=True)
class BulkExportResult:
    status: ExportStatus
    message: str
    filename: str | None = None
    zip_base64: str | None = None
    exported_count: int = 0
    failed_count: int = 0
    failures: list[dict[str, str]] | None = None

    def zip_bytes(self) -> bytes:
        return base64.b64decode(self.zip_base64 or "")

    def to_dict(self) -> dict[str, object]:
        return {
            "status": self.status.value,
            "message": self.message,
            "filename": self.filename,
            "zip_base64": self.zip_base64,
            "exported_count": self.exported_count,
            "failed_count": self.failed_count,
            "failures": self.failures or [],
        }
