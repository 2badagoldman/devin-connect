import argparse
import json
import shutil
from pathlib import Path
from typing import Any


def collect_notices(metadata_path: Path, destination: Path) -> int:
    metadata: dict[str, Any] = json.loads(metadata_path.read_text(encoding="utf-8"))
    resolved: set[str] = {node["id"] for node in metadata["resolve"]["nodes"]}
    count = 0
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in resolved or package["source"] is None:
            continue
        root = Path(package["manifest_path"]).parent.resolve()
        for path in sorted(root.rglob("*")):
            if not path.is_file() or not path.name.upper().startswith(
                ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")
            ):
                continue
            if not path.resolve().is_relative_to(root):
                raise ValueError(f"notice points outside package: {path}")
            target = destination / f"{package['name']}-{package['version']}" / path.relative_to(root)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            count += 1
    return count


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("metadata", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    print(f"Copied {collect_notices(args.metadata, args.destination)} upstream license and notice files")
