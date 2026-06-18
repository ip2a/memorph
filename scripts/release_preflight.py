#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    cargo_toml = (ROOT / "rust" / "crates" / "memorph" / "Cargo.toml").read_text(encoding="utf-8")
    return tomllib.loads(cargo_toml)["package"]


def read_required_string(data: dict[str, object], key: str) -> str:
    value = data.get(key)
    if not isinstance(value, str):
        raise SystemExit(f"[error] Cargo.toml package.{key} is missing")
    return value


def validate_cargo_metadata(cargo_package: dict[str, object]) -> None:
    package_name = read_required_string(cargo_package, "name")
    if package_name != "memorph":
        raise SystemExit(f"[error] Crates.io package name must be memorph, got {package_name}")
    read_required_string(cargo_package, "version")
    read_required_string(cargo_package, "repository")
    read_required_string(cargo_package, "readme")
    print("[ok] Cargo package metadata is complete for release")


def validate_cargo_targets() -> None:
    cargo_path = ROOT / "rust" / "crates" / "memorph" / "Cargo.toml"
    cargo = tomllib.loads(cargo_path.read_text(encoding="utf-8"))
    bin_targets = {
        item.get("name"): item.get("path")
        for item in cargo.get("bin", [])
        if isinstance(item, dict)
    }
    expected = {
        "memorph": "src/bin/memorph.rs",
        "memo": "src/bin/memo.rs",
    }
    if bin_targets != expected:
        raise SystemExit(
            f"[error] Cargo binary targets mismatch in {cargo_path}; expected={expected} actual={bin_targets}"
        )
    for path in expected.values():
        if not (cargo_path.parent / path).exists():
            raise SystemExit(f"[error] Missing Cargo binary source: {cargo_path.parent / path}")
    if cargo["package"].get("default-run") != "memorph":
        raise SystemExit(f"[error] Cargo package.default-run must be memorph in {cargo_path}")
    print("[ok] Crates.io package exposes library and CLI binaries")


def load_platform_config() -> tuple[dict[str, str], list[dict[str, str]], list[dict[str, object]]]:
    data = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
    return data["meta"], data["platforms"], data.get("desktop_targets", [])


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


def validate_desktop_targets(desktop_targets: list[dict[str, object]]) -> None:
    seen: set[tuple[str, str]] = set()
    for target in desktop_targets:
        key = (str(target["platform_id"]), str(target["bundle"]))
        if key in seen:
            raise SystemExit(f"[error] Duplicate desktop target: {key[0]} bundle={key[1]}")
        seen.add(key)
    print(f"[ok] Desktop target matrix has {len(desktop_targets)} unique bundle targets")


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


def validate_desktop_metadata(version: str, package_name: str, desktop_targets: list[dict[str, object]]) -> None:
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
    bundle_config = tauri_config.get("bundle", {})
    bundle_targets = bundle_config.get("targets", [])
    expected_bundles = {str(target["bundle"]) for target in desktop_targets}
    if set(bundle_targets) != expected_bundles:
        raise SystemExit(
            f"[error] desktop bundle.targets mismatch in {tauri_config_path}; expected={sorted(expected_bundles)} actual={sorted(bundle_targets)}"
        )
    icons = bundle_config.get("icon", [])
    if not isinstance(icons, list):
        raise SystemExit(f"[error] desktop bundle.icon must be a list in {tauri_config_path}")
    for icon in icons:
        icon_path = ROOT / "desktop" / "tauri" / str(icon)
        if not icon_path.exists():
            raise SystemExit(f"[error] Missing desktop icon asset: {icon_path}")
    print("[ok] Desktop metadata matches Cargo version")


def validate_desktop_workflows(desktop_targets: list[dict[str, object]]) -> None:
    expected: dict[str, dict[str, object]] = {}
    for item in desktop_targets:
        platform_id = str(item["platform_id"])
        runner = str(item["runner"])
        target = str(item["target"])
        bundle = str(item["bundle"])
        current = expected.setdefault(
            platform_id,
            {"runner": runner, "target": target, "bundles": set()},
        )
        if current["runner"] != runner or current["target"] != target:
            raise SystemExit(f"[error] Inconsistent desktop target metadata for {platform_id}")
        current["bundles"].add(bundle)

    build_workflow_path = ROOT / ".github" / "workflows" / "release-build-desktop.yml"
    build_workflow = build_workflow_path.read_text(encoding="utf-8")
    pattern = re.compile(
        r"^\s*-\s+platform_id:\s*(?P<platform_id>\S+)\n"
        r"\s+runner:\s*(?P<runner>\S+)\n"
        r"\s+target:\s*(?P<target>\S+)\n"
        r"\s+bundles:\s*(?P<bundles>[^\n]+)",
        re.MULTILINE,
    )
    actual: dict[str, dict[str, object]] = {}
    for match in pattern.finditer(build_workflow):
        platform_id = match.group("platform_id")
        actual[platform_id] = {
            "runner": match.group("runner"),
            "target": match.group("target"),
            "bundles": {item.strip() for item in match.group("bundles").split(",") if item.strip()},
        }

    if set(actual) != set(expected):
        missing = sorted(set(expected) - set(actual))
        extra = sorted(set(actual) - set(expected))
        raise SystemExit(
            f"[error] Desktop build workflow platforms mismatch; missing={missing or '[]'} extra={extra or '[]'}"
        )

    for platform_id, info in expected.items():
        actual_info = actual[platform_id]
        if actual_info["runner"] != info["runner"] or actual_info["target"] != info["target"]:
            raise SystemExit(
                f"[error] Desktop build workflow target mismatch for {platform_id}; "
                f"expected runner={info['runner']} target={info['target']}"
            )
        if actual_info["bundles"] != info["bundles"]:
            raise SystemExit(
                f"[error] Desktop build workflow bundles mismatch for {platform_id}; "
                f"expected={sorted(info['bundles'])} actual={sorted(actual_info['bundles'])}"
            )

    verify_workflow_path = ROOT / ".github" / "workflows" / "post-release-verify.yml"
    verify_workflow = verify_workflow_path.read_text(encoding="utf-8")
    required_fragments = [
        "verify-desktop-dmg:",
        "desktop-release-assets/*.deb",
        "desktop-release-assets/*.AppImage",
        "desktop-release-assets/*.exe",
        "desktop-release-assets/*.dmg",
    ]
    for fragment in required_fragments:
        if fragment not in verify_workflow:
            raise SystemExit(
                f"[error] Desktop post-release verify workflow missing required fragment: {fragment}"
            )

    print("[ok] Desktop workflows match the configured platform matrix")


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
    version = read_required_string(cargo_package, "version")
    repository_url = read_required_string(cargo_package, "repository")
    validate_cargo_metadata(cargo_package)
    validate_cargo_targets()
    meta, platforms, desktop_targets = load_platform_config()
    if meta.get("version_source") != "Cargo.toml":
        raise SystemExit("[error] platforms.toml meta.version_source must be Cargo.toml")
    package_name = meta.get("binary_name")
    if not package_name:
        raise SystemExit("[error] platforms.toml meta.binary_name is required")

    validate_unique_platforms(platforms)
    validate_desktop_targets(desktop_targets)
    validate_npm_packages(package_name, version, repository_url, platforms)
    validate_python_packages(package_name, version, platforms)
    validate_desktop_metadata(version, package_name, desktop_targets)
    validate_desktop_workflows(desktop_targets)
    if not args.skip_sync_check:
        validate_version_sync()
    if args.dist_root:
        validate_dist((ROOT / args.dist_root).resolve(), platforms)

    print(f"[ok] Release preflight passed for v{version}")


if __name__ == "__main__":
    main()
