#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    cargo_toml = (ROOT / "rust" / "crates" / "memorph" / "Cargo.toml").read_text(encoding="utf-8")
    return tomllib.loads(cargo_toml)["package"]


def read_cargo_version() -> str:
    version = read_cargo_package().get("version")
    if not isinstance(version, str):
        raise RuntimeError("Version not found in Cargo.toml")
    return version


def read_cargo_repository() -> str:
    repository = read_cargo_package().get("repository")
    if not isinstance(repository, str):
        raise RuntimeError("Repository not found in Cargo.toml")
    return repository


def write_or_check(path: Path, content: str, check: bool) -> None:
    if check:
        current = path.read_text(encoding="utf-8")
        if current != content:
            raise RuntimeError(f"File is not version-synced: {path.relative_to(ROOT)}")
        return
    path.write_text(content, encoding="utf-8")


def update_json_version(path: Path, version: str, repository_url: str, check: bool) -> None:
    data = json.loads(path.read_text(encoding="utf-8"))
    data["version"] = version
    repository = data.get("repository")
    if not isinstance(repository, dict):
        repository = {"type": "git"}
    repository["type"] = "git"
    repository["url"] = repository_url
    data["repository"] = repository
    if "optionalDependencies" in data:
        for name in data["optionalDependencies"]:
            data["optionalDependencies"][name] = version
    write_or_check(path, json.dumps(data, ensure_ascii=False, indent=2) + "\n", check)


def update_pyproject_version(path: Path, version: str, check: bool) -> None:
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    in_project = False
    replaced = False
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[project]":
            in_project = True
            continue
        if stripped.startswith("[") and stripped != "[project]":
            in_project = False
        if in_project and stripped.startswith("version ="):
            lines[idx] = f'version = "{version}"\n'
            replaced = True
    if not replaced:
        raise RuntimeError(f"Missing [project].version in file: {path}")
    write_or_check(path, "".join(lines), check)


def update_toml_package_version(path: Path, version: str, check: bool) -> None:
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    in_package = False
    replaced = False
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[package]":
            in_package = True
            continue
        if stripped.startswith("[") and stripped != "[package]":
            in_package = False
        if in_package and stripped.startswith("version ="):
            lines[idx] = f'version = "{version}"\n'
            replaced = True
    if not replaced:
        raise RuntimeError(f"Missing [package].version in file: {path}")
    write_or_check(path, "".join(lines), check)


def update_memorph_app_dependency_version(path: Path, version: str, check: bool) -> None:
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    replaced = False
    for idx, line in enumerate(lines):
        if not line.lstrip().startswith("memorph-lib = {"):
            continue
        if not re.search(r'version = "[^"]+"', line):
            raise RuntimeError(f"Missing memorph-lib dependency version in file: {path}")
        updated = re.sub(r'version = "[^"]+"', f'version = "{version}"', line, count=1)
        lines[idx] = updated
        replaced = True
        break
    if not replaced:
        raise RuntimeError(f"Missing memorph-lib dependency in file: {path}")
    write_or_check(path, "".join(lines), check)


def update_tauri_config_version(path: Path, version: str, check: bool) -> None:
    data = json.loads(path.read_text(encoding="utf-8"))
    data["version"] = version
    write_or_check(path, json.dumps(data, ensure_ascii=False, indent=2) + "\n", check)


def load_platforms() -> list[dict[str, str]]:
    platforms_toml = (ROOT / "platforms.toml").read_text(encoding="utf-8")
    return tomllib.loads(platforms_toml)["platforms"]


def update_memorph_python_dependencies(version: str, check: bool) -> None:
    path = ROOT / "python" / "packages" / "memorph" / "pyproject.toml"
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)

    in_project = False
    dep_start = None
    dep_end = None
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[project]":
            in_project = True
            continue
        if stripped.startswith("[") and stripped != "[project]":
            in_project = False
        if in_project and stripped.startswith("dependencies = ["):
            dep_start = idx
            continue
        if dep_start is not None and dep_end is None and stripped == "]":
            dep_end = idx
            break

    if dep_start is None or dep_end is None:
        raise RuntimeError(f"Could not find [project].dependencies block in: {path}")

    dep_indent = "  "
    new_deps = []
    for platform in load_platforms():
        package = platform["python_package"]
        marker = platform["python_marker"]
        new_deps.append(f'{dep_indent}"{package}=={version}; {marker}",\n')

    lines[dep_start + 1 : dep_end] = new_deps
    write_or_check(path, "".join(lines), check)
    print(f"[ok] Main package dependency versions are aligned: {path.relative_to(ROOT)}")


def sync_i18n_asset(check: bool) -> None:
    source = ROOT / "web" / "i18n.json"
    dest = ROOT / "rust" / "crates" / "memorph" / "assets" / "i18n.json"
    if check:
        if not dest.exists():
            raise RuntimeError(f"Missing i18n asset: {dest.relative_to(ROOT)}")
        if source.read_bytes() != dest.read_bytes():
            raise RuntimeError(f"i18n asset out of sync: {dest.relative_to(ROOT)}")
        print("[ok] i18n asset is synced")
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, dest)
    print(f"[ok] i18n asset synced: {dest.relative_to(ROOT)}")


def sync_cargo_lockfile(check: bool, check_lockfile: bool) -> None:
    rust_root = ROOT / "rust"
    if check and check_lockfile:
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=rust_root,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=ROOT / "desktop" / "tauri",
            check=True,
            stdout=subprocess.DEVNULL,
        )
        print("[ok] Cargo.lock is up to date")
        return
    if check:
        return
    subprocess.run(["cargo", "generate-lockfile"], cwd=rust_root, check=True)
    subprocess.run(["cargo", "generate-lockfile"], cwd=ROOT / "desktop" / "tauri", check=True)
    print("[ok] Synced Cargo.lock")


def main() -> None:
    parser = argparse.ArgumentParser(description="Sync package versions from Cargo.toml")
    parser.add_argument("--check", action="store_true", help="fail if generated metadata is not already synced")
    parser.add_argument(
        "--check-lockfile",
        action="store_true",
        help="with --check, also verify Cargo.lock using cargo metadata --locked",
    )
    args = parser.parse_args()

    version = read_cargo_version()
    repository_url = read_cargo_repository()
    print(f"[info] Unified version: {version}")

    npm_packages_dir = ROOT / "npm" / "packages"
    if not npm_packages_dir.exists():
        raise RuntimeError(f"Missing npm packages directory: {npm_packages_dir}")
    json_files = sorted(npm_packages_dir.rglob("package.json"))
    if not json_files:
        raise RuntimeError(f"No npm package.json files found under: {npm_packages_dir}")
    for json_file in json_files:
        update_json_version(json_file, version, repository_url, args.check)
        print(f"[ok] Version aligned: {json_file.relative_to(ROOT)}")

    pyproject_files = sorted((ROOT / "python" / "packages").rglob("pyproject.toml"))
    for pyproject_file in pyproject_files:
        update_pyproject_version(pyproject_file, version, args.check)
        print(f"[ok] Version aligned: {pyproject_file.relative_to(ROOT)}")

    desktop_cargo = ROOT / "desktop" / "tauri" / "Cargo.toml"
    update_toml_package_version(desktop_cargo, version, args.check)
    print(f"[ok] Version aligned: {desktop_cargo.relative_to(ROOT)}")

    app_cargo = ROOT / "rust" / "apps" / "memorph" / "Cargo.toml"
    update_toml_package_version(app_cargo, version, args.check)
    print(f"[ok] Version aligned: {app_cargo.relative_to(ROOT)}")
    update_memorph_app_dependency_version(app_cargo, version, args.check)
    print(f"[ok] App crate dependency aligned: {app_cargo.relative_to(ROOT)}")

    tauri_config = ROOT / "desktop" / "tauri" / "tauri.conf.json"
    update_tauri_config_version(tauri_config, version, args.check)
    print(f"[ok] Version aligned: {tauri_config.relative_to(ROOT)}")

    update_memorph_python_dependencies(version, args.check)
    sync_i18n_asset(args.check)
    sync_cargo_lockfile(args.check, args.check_lockfile)


if __name__ == "__main__":
    main()
