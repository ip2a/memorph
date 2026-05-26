#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    return tomllib.loads((ROOT / "rust" / "crates" / "memorph" / "Cargo.toml").read_text(encoding="utf-8"))["package"]


def load_desktop_targets() -> list[dict[str, object]]:
    return tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))["desktop_targets"]


def select_desktop_targets(platform_ids: list[str]) -> list[dict[str, object]]:
    targets = load_desktop_targets()
    if not platform_ids:
        return targets
    selected = [target for target in targets if str(target["platform_id"]) in platform_ids]
    if not selected:
        raise SystemExit(f"[error] No desktop targets matched platform filters: {platform_ids}")
    return selected


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_checksums(path: Path) -> dict[str, str]:
    checksums: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, filename = line.split("  ", 1)
        checksums[filename] = digest
    return checksums


def main() -> None:
    parser = argparse.ArgumentParser(description="Verify desktop release assets")
    parser.add_argument("--assets-dir", default="desktop-release-assets", help="release asset directory")
    parser.add_argument("--version", help="released version; defaults to Cargo.toml version")
    parser.add_argument(
        "--platform-id",
        action="append",
        default=[],
        help="limit verification to one or more desktop platform ids",
    )
    args = parser.parse_args()

    cargo_package = read_cargo_package()
    config = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
    meta = config["meta"]
    desktop_targets = select_desktop_targets(args.platform_id)
    binary_name = str(meta.get("binary_name", cargo_package["name"]))
    version = args.version or str(cargo_package["version"])
    assets_dir = (ROOT / args.assets_dir).resolve()
    checksum_path = assets_dir / "SHA256SUMS-desktop.txt"

    if not checksum_path.exists():
        raise SystemExit(f"[error] Missing desktop checksum file: {checksum_path}")

    expected_names = {
        f"{binary_name}-desktop-v{version}-{target['platform_id']}{target['extension']}"
        for target in desktop_targets
    }
    actual_names = {
        path.name
        for path in assets_dir.iterdir()
        if path.is_file() and path.name != "SHA256SUMS-desktop.txt"
    }
    if actual_names != expected_names:
        missing = sorted(expected_names - actual_names)
        extra = sorted(actual_names - expected_names)
        raise SystemExit(
            f"[error] Desktop release assets mismatch; missing={missing or '[]'} extra={extra or '[]'}"
        )

    checksums = parse_checksums(checksum_path)
    if set(checksums) != expected_names:
        raise SystemExit("[error] SHA256SUMS-desktop.txt does not match expected desktop assets")

    for filename in expected_names:
        path = assets_dir / filename
        if checksums[filename] != sha256(path):
            raise SystemExit(f"[error] SHA256 mismatch for {filename}")

    print(f"[ok] Verified {len(expected_names)} desktop release assets for v{version}")


if __name__ == "__main__":
    main()
