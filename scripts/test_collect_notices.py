import json
import runpy
import tempfile
import unittest
from pathlib import Path
from typing import Any


class NoticeCollectionTests(unittest.TestCase):
    def test_preserves_notices_only_from_resolved_external_packages(self) -> None:
        collect = runpy.run_path(str(Path(__file__).with_name("collect-notices.py")))["collect_notices"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            packages: list[dict[str, Any]] = []
            for name, source in [("dependency", "registry"), ("unused", "registry"), ("local", None)]:
                package = root / name
                package.mkdir()
                (package / "NOTICE").write_bytes(f"Attribution for {name}\r\n".encode())
                packages.append({
                    "id": name,
                    "name": name,
                    "version": "1.2.3",
                    "source": source,
                    "manifest_path": str(package / "Cargo.toml"),
                })
            nested = root / "dependency" / "nested"
            nested.mkdir()
            (nested / "Copyright.txt").write_bytes(b"Nested copyright\n")
            (root / "dependency" / "COPYING").write_bytes(b"Original license text\n")
            (nested / "source.rs").write_text("not a notice", encoding="utf-8")
            metadata = root / "metadata.json"
            metadata.write_text(json.dumps({
                "packages": packages,
                "resolve": {"nodes": [{"id": "dependency"}, {"id": "local"}]},
            }), encoding="utf-8")
            destination = root / "output"

            self.assertEqual(collect(metadata, destination), 3)
            self.assertEqual(collect(metadata, destination), 3)
            self.assertEqual(
                {str(path.relative_to(destination)) for path in destination.rglob("*") if path.is_file()},
                {"dependency-1.2.3/NOTICE", "dependency-1.2.3/nested/Copyright.txt", "dependency-1.2.3/COPYING"},
            )
            self.assertEqual((destination / "dependency-1.2.3/NOTICE").read_bytes(), b"Attribution for dependency\r\n")
            self.assertEqual((destination / "dependency-1.2.3/nested/Copyright.txt").read_bytes(), b"Nested copyright\n")
            self.assertEqual((destination / "dependency-1.2.3/COPYING").read_bytes(), b"Original license text\n")

            (root / "dependency" / "NOTICE-external").symlink_to(root / "local" / "NOTICE")
            with self.assertRaisesRegex(ValueError, "outside package"):
                collect(metadata, destination)
            self.assertFalse((destination / "dependency-1.2.3/NOTICE-external").exists())


if __name__ == "__main__":
    unittest.main()
