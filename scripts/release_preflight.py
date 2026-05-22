#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    cargo_toml = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    return tomllib.loads(cargo_toml)["package"]


def read_required_string(data: dict[str, object], key: str) -> str:
    value = data.get(key)
    if not isinstance(value, str):
        raise SystemExit(f"[error] Cargo.toml package.{key} is missing")
    return value


def validate_cargo_metadata(cargo_package: dict[str, object]) -> None:
    read_required_string(cargo_package, "name")
    read_required_string(cargo_package, "version")
    read_required_string(cargo_package, "repository")
    read_required_string(cargo_package, "readme")
    print("[ok] Cargo package metadata is complete for release")


def load_platform_config() -> tuple[dict[str, str], list[dict[str, str]]]:
    data = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
    return data["meta"], data["platforms"]


def read_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_unique_platforms(platforms: list[dict[str, str]]) -> None:
    seen_ids: set[str] = set()
    seen_npm: set[str] = set()
    seen_python: set[str] = set()
    for platform in platforms:
        platform_id = platform["id"]
        npm_package = platform["npm_package"]
        python_package = platform["python_package"]
        if platform_id in seen_ids:
            raise SystemExit(f"[error] Duplicate platform id: {platform_id}")
        if npm_package in seen_npm:
            raise SystemExit(f"[error] Duplicate npm package: {npm_package}")
        if python_package in seen_python:
            raise SystemExit(f"[error] Duplicate Python package: {python_package}")
        seen_ids.add(platform_id)
        seen_npm.add(npm_package)
        seen_python.add(python_package)
    print(f"[ok] Platform matrix has {len(platforms)} unique platforms")


def validate_npm_packages(package_name: str, version: str, repository_url: str, platforms: list[dict[str, str]]) -> None:
    packages_dir = ROOT / "npm" / "packages"
    main_path = packages_dir / package_name / "package.json"
    main = read_json(main_path)
    if main.get("name") != package_name:
        raise SystemExit(f"[error] npm main package name mismatch in {main_path}")
    if main.get("version") != version:
        raise SystemExit(f"[error] npm main package version mismatch in {main_path}")

    expected_optional = {platform["npm_package"]: version for platform in platforms}
    if main.get("optionalDependencies") != expected_optional:
        raise SystemExit("[error] npm optionalDependencies do not match platforms.toml")

    for platform in platforms:
        package_path = packages_dir / platform["npm_dir"] / "package.json"
        data = read_json(package_path)
        if data.get("name") != platform["npm_package"]:
            raise SystemExit(f"[error] npm package name mismatch in {package_path}")
        if data.get("version") != version:
            raise SystemExit(f"[error] npm package version mismatch in {package_path}")
        repository = data.get("repository")
        if not isinstance(repository, dict) or repository.get("url") != repository_url:
            raise SystemExit(f"[error] npm repository URL mismatch in {package_path}")
        if data.get("os") != [platform["os"]]:
            raise SystemExit(f"[error] npm os mismatch in {package_path}")
        if data.get("cpu") != [platform["cpu"]]:
            raise SystemExit(f"[error] npm cpu mismatch in {package_path}")

    print("[ok] npm package metadata matches platforms.toml")


def validate_python_packages(package_name: str, version: str, platforms: list[dict[str, str]]) -> None:
    packages_dir = ROOT / "python" / "packages"
    main_path = packages_dir / package_name / "pyproject.toml"
    main = tomllib.loads(main_path.read_text(encoding="utf-8"))
    if main["project"]["name"] != package_name:
        raise SystemExit(f"[error] Python main package name mismatch in {main_path}")
    if main["project"]["version"] != version:
        raise SystemExit(f"[error] Python main package version mismatch in {main_path}")

    expected_deps = [
        f'{platform["python_package"]}=={version}; {platform["python_marker"]}'
        for platform in platforms
    ]
    if main["project"].get("dependencies") != expected_deps:
        raise SystemExit("[error] Python platform dependencies do not match platforms.toml")

    for platform in platforms:
        package_path = packages_dir / platform["python_package"] / "pyproject.toml"
        data = tomllib.loads(package_path.read_text(encoding="utf-8"))
        if data["project"]["name"] != platform["python_package"]:
            raise SystemExit(f"[error] Python package name mismatch in {package_path}")
        if data["project"]["version"] != version:
            raise SystemExit(f"[error] Python package version mismatch in {package_path}")

    print("[ok] Python package metadata matches platforms.toml")


def validate_dist(dist_root: Path, platforms: list[dict[str, str]]) -> None:
    for platform in platforms:
        binary = dist_root / platform["id"] / platform["artifact_binary"]
        checksum = dist_root / platform["id"] / "SHA256SUMS"
        if not binary.exists():
            raise SystemExit(f"[error] Missing dist binary: {binary}")
        if not checksum.exists():
            raise SystemExit(f"[error] Missing dist checksum: {checksum}")
    print(f"[ok] dist artifacts exist under {dist_root}")


def validate_desktop_metadata(version: str, package_name: str) -> None:
    desktop_cargo_path = ROOT / "desktop" / "tauri" / "Cargo.toml"
    desktop_cargo = tomllib.loads(desktop_cargo_path.read_text(encoding="utf-8"))
    desktop_package = desktop_cargo["package"]
    desktop_version = desktop_package.get("version")
    if desktop_version != version:
        raise SystemExit(f"[error] desktop Tauri Cargo version mismatch in {desktop_cargo_path}")

    tauri_config_path = ROOT / "desktop" / "tauri" / "tauri.conf.json"
    tauri_config = read_json(tauri_config_path)
    if tauri_config.get("version") != version:
        raise SystemExit(f"[error] desktop tauri.conf version mismatch in {tauri_config_path}")
    if tauri_config.get("productName") != package_name:
        raise SystemExit(f"[error] desktop productName mismatch in {tauri_config_path}")
    print("[ok] Desktop metadata matches Cargo version")


def validate_version_sync() -> None:
    subprocess.run(
        ["python3", "scripts/sync_version.py", "--check"],
        cwd=ROOT,
        check=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Run trusted release metadata preflight checks")
    parser.add_argument("--dist-root", help="optional dist root to validate")
    parser.add_argument("--skip-sync-check", action="store_true")
    args = parser.parse_args()

    cargo_package = read_cargo_package()
    package_name = read_required_string(cargo_package, "name")
    version = read_required_string(cargo_package, "version")
    repository_url = read_required_string(cargo_package, "repository")
    validate_cargo_metadata(cargo_package)
    meta, platforms = load_platform_config()
    if meta.get("version_source") != "Cargo.toml":
        raise SystemExit("[error] platforms.toml meta.version_source must be Cargo.toml")
    if not meta.get("binary_name"):
        raise SystemExit("[error] platforms.toml meta.binary_name is required")

    validate_unique_platforms(platforms)
    validate_npm_packages(package_name, version, repository_url, platforms)
    validate_python_packages(package_name, version, platforms)
    validate_desktop_metadata(version, package_name)
    if not args.skip_sync_check:
        validate_version_sync()
    if args.dist_root:
        validate_dist((ROOT / args.dist_root).resolve(), platforms)

    print(f"[ok] Release preflight passed for v{version}")


if __name__ == "__main__":
    main()
