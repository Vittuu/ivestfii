from __future__ import annotations

import json
from datetime import datetime
from pathlib import Path
from typing import Any, Dict

DEFAULT_DATA_PATH = Path(__file__).resolve().parent.parent / "data" / "fiis_data.json"


def ensure_data_file(path: Path = DEFAULT_DATA_PATH) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        save_data({"fiis": []}, path)


def load_data(path: Path = DEFAULT_DATA_PATH) -> Dict[str, Any]:
    ensure_data_file(path)
    with path.open("r", encoding="utf-8") as handle:
        try:
            content = json.load(handle)
        except json.JSONDecodeError:
            content = {"fiis": []}
    content.setdefault("fiis", [])
    return content


def save_data(data: Dict[str, Any], path: Path = DEFAULT_DATA_PATH) -> None:
    ensure_data_file(path)
    with path.open("w", encoding="utf-8") as handle:
        json.dump(data, handle, indent=2, ensure_ascii=False)


def create_backup(path: Path = DEFAULT_DATA_PATH) -> Path:
    ensure_data_file(path)
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    backup_path = path.parent / f"fiis_backup_{timestamp}.json"
    data = load_data(path)
    save_data(data, backup_path)
    return backup_path

