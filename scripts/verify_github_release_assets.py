#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import tarfile
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    cargo_toml = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    return tomllib.loads(cargo_toml)["package"]


def load_platform_config() -> tuple[dict[str, str], list[dict[str, str]]]:
    data = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
    return data["meta"], data["platforms"]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expected_archive_name(release_name: str, version: str, platform: dict[str, str]) -> str:
    suffix = ".zip" if platform["id"].startswith("win32-") else ".tar.gz"
    return f"{release_name}-v{version}-{platform['id']}{suffix}"


def parse_top_level_checksums(path: Path) -> dict[str, str]:
    checksums: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, filename = line.split("  ", 1)
        checksums[filename] = digest
    return checksums


def verify_tar_asset(path: Path, binary_name: str) -> None:
    with tarfile.open(path, "r:gz") as archive:
        names = set(archive.getnames())
        required = {binary_name, "SHA256SUMS", "README.md", "LICENSE"}
        missing = required - names
        if missing:
            raise SystemExit(f"[error] Missing tar members in {path.name}: {sorted(missing)}")
        member = archive.getmember(binary_name)
        if member.mode != 0o755:
            raise SystemExit(
                f"[error] Binary mode in {path.name} is {oct(member.mode)}, expected 0o755"
            )


def verify_zip_asset(path: Path, binary_name: str) -> None:
    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        required = {binary_name, "SHA256SUMS", "README.md", "LICENSE"}
        missing = required - names
        if missing:
            raise SystemExit(f"[error] Missing zip members in {path.name}: {sorted(missing)}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Verify downloaded GitHub Release assets")
    parser.add_argument("--assets-dir", default="release-assets", help="directory containing GitHub Release assets")
    parser.add_argument("--version", help="released version; defaults to Cargo.toml version")
    args = parser.parse_args()

    cargo_package = read_cargo_package()
    version = args.version or cargo_package.get("version")
    if not isinstance(version, str):
        raise SystemExit("[error] Cargo.toml package.version is missing")

    meta, platforms = load_platform_config()
    release_name = meta.get("binary_name")
    if not isinstance(release_name, str):
        raise SystemExit("[error] platforms.toml meta.binary_name is missing")

    assets_dir = (ROOT / args.assets_dir).resolve()
    top_level_checksum_path = assets_dir / "SHA256SUMS.txt"
    if not top_level_checksum_path.exists():
        raise SystemExit(f"[error] Missing top-level checksum file: {top_level_checksum_path}")

    expected_names = {
        expected_archive_name(release_name, version, platform): platform for platform in platforms
    }
    actual_files = {
        path.name
        for path in assets_dir.iterdir()
        if path.is_file() and path.name != "SHA256SUMS.txt"
    }
    if actual_files != set(expected_names):
        missing = sorted(set(expected_names) - actual_files)
        extra = sorted(actual_files - set(expected_names))
        raise SystemExit(
            f"[error] Release assets mismatch; missing={missing or '[]'} extra={extra or '[]'}"
        )

    checksums = parse_top_level_checksums(top_level_checksum_path)
    if set(checksums) != set(expected_names):
        raise SystemExit("[error] Top-level SHA256SUMS.txt does not match expected asset set")

    for filename, platform in expected_names.items():
        path = assets_dir / filename
        digest = sha256(path)
        if checksums[filename] != digest:
            raise SystemExit(f"[error] SHA256 mismatch for {filename}")
        binary_name = platform["artifact_binary"]
        if path.suffix == ".zip":
            verify_zip_asset(path, binary_name)
        else:
            verify_tar_asset(path, binary_name)

    print(f"[ok] Verified {len(expected_names)} GitHub Release assets for v{version}")


if __name__ == "__main__":
    main()
