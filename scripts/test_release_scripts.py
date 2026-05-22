#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run_command(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        **kwargs,
    )


class ReleaseScriptsTest(unittest.TestCase):
    def test_release_metadata_preflight_passes(self) -> None:
        result = run_command(["python3", "scripts/release_preflight.py"])
        self.assertIn("[ok] Release preflight passed", result.stdout)

    def test_sync_version_check_passes_without_writing(self) -> None:
        result = run_command(["python3", "scripts/sync_version.py", "--check"])
        self.assertIn("[ok] Main package dependency versions are aligned", result.stdout)

    def test_release_readiness_report_contains_current_repository(self) -> None:
        result = run_command(["python3", "scripts/release_readiness.py", "--json"])
        report = json.loads(result.stdout)
        self.assertEqual(report["package_name"], "memorph")
        self.assertEqual(report["repository"], "https://github.com/ip2a/memorph")
        self.assertEqual(
            report["workflows"]["publish_npm"],
            ".github/workflows/release-publish-npm.yml",
        )

    def test_prepare_github_release_assets_from_dist(self) -> None:
        config = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
        platforms = config["platforms"]

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            dist_root = tmp_path / "dist"
            assets_dir = tmp_path / "release-assets"

            for platform in platforms:
                platform_dir = dist_root / platform["id"]
                platform_dir.mkdir(parents=True)
                binary = platform_dir / platform["artifact_binary"]
                binary.write_bytes(f"fake binary {platform['id']}".encode("utf-8"))
                binary.chmod(0o755)
                digest = hashlib.sha256(binary.read_bytes()).hexdigest()
                (platform_dir / "SHA256SUMS").write_text(
                    f"{digest}  {platform['artifact_binary']}\n",
                    encoding="utf-8",
                )

            run_command(
                [
                    "python3",
                    "scripts/prepare_github_release_assets.py",
                    "--dist-root",
                    str(dist_root),
                    "--assets-dir",
                    str(assets_dir),
                ]
            )
            run_command(
                [
                    "python3",
                    "scripts/release_preflight.py",
                    "--dist-root",
                    str(dist_root),
                    "--skip-sync-check",
                ]
            )
            run_command(
                [
                    "python3",
                    "scripts/verify_github_release_assets.py",
                    "--assets-dir",
                    str(assets_dir),
                ]
            )

            release_assets = sorted(path.name for path in assets_dir.iterdir())
            self.assertEqual(len(release_assets), len(platforms) + 1)
            self.assertIn("SHA256SUMS.txt", release_assets)

            checksum_lines = (assets_dir / "SHA256SUMS.txt").read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(checksum_lines), len(platforms))
            for line in checksum_lines:
                digest, filename = line.split("  ", 1)
                self.assertEqual(len(digest), 64)
                self.assertTrue((assets_dir / filename).exists())

            tar_platform = next(platform for platform in platforms if not platform["id"].startswith("win32-"))
            tar_asset = next(path for path in assets_dir.iterdir() if path.name.endswith(".tar.gz"))
            with tarfile.open(tar_asset, "r:gz") as archive:
                member = archive.getmember(tar_platform["artifact_binary"])
                self.assertEqual(member.mode, 0o755)

            zip_platform = next(platform for platform in platforms if platform["id"].startswith("win32-"))
            zip_asset = next(path for path in assets_dir.iterdir() if path.suffix == ".zip")
            with zipfile.ZipFile(zip_asset) as archive:
                self.assertIn(zip_platform["artifact_binary"], archive.namelist())
                self.assertIn("SHA256SUMS", archive.namelist())

    def test_prepare_desktop_release_assets(self) -> None:
        version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            target_root = tmp_path / "desktop" / "tauri" / "target"
            assets_dir = tmp_path / "desktop-release-assets"
            bundle_dir = target_root / "aarch64-apple-darwin" / "release" / "bundle" / "dmg"
            bundle_dir.mkdir(parents=True)
            source = bundle_dir / f"memorph_{version}_aarch64.dmg"
            source.write_bytes(b"fake dmg")

            run_command(
                [
                    "python3",
                    "scripts/prepare_desktop_release_assets.py",
                    "--target-root",
                    str(target_root),
                    "--assets-dir",
                    str(assets_dir),
                ]
            )
            run_command(
                [
                    "python3",
                    "scripts/verify_desktop_release_assets.py",
                    "--assets-dir",
                    str(assets_dir),
                    "--version",
                    version,
                ]
            )

            expected_asset = assets_dir / f"memorph-desktop-v{version}-darwin-arm64.dmg"
            self.assertTrue(expected_asset.exists())
            checksum_file = assets_dir / "SHA256SUMS-desktop.txt"
            self.assertTrue(checksum_file.exists())
            self.assertIn(expected_asset.name, checksum_file.read_text(encoding="utf-8"))

    def test_generate_latest_release_manifest(self) -> None:
        version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            release_assets = tmp_path / "release-assets"
            desktop_assets = tmp_path / "desktop-release-assets"
            release_assets.mkdir()
            desktop_assets.mkdir()
            (release_assets / f"memorph-v{version}-linux-x64-gnu.tar.gz").write_bytes(b"cli")
            (release_assets / "SHA256SUMS.txt").write_text("checksum\n", encoding="utf-8")
            (desktop_assets / f"memorph-desktop-v{version}-darwin-arm64.dmg").write_bytes(b"dmg")
            (desktop_assets / "SHA256SUMS-desktop.txt").write_text("checksum\n", encoding="utf-8")
            output = tmp_path / "latest.json"

            run_command(
                [
                    "python3",
                    "scripts/generate_latest_release_manifest.py",
                    "--version",
                    version,
                    "--tag",
                    f"v{version}",
                    "--base-url",
                    f"https://github.com/ip2a/memorph/releases/download/v{version}",
                    "--assets-dir",
                    str(release_assets),
                    "--assets-dir",
                    str(desktop_assets),
                    "--output",
                    str(output),
                ]
            )
            payload = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(payload["version"], f"v{version}")
            self.assertTrue(any(asset["name"].endswith(".dmg") for asset in payload["assets"]))

    def test_install_script_installs_from_release_assets(self) -> None:
        config = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
        version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
        platforms = config["platforms"]

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            dist_root = tmp_path / "dist"
            assets_dir = tmp_path / "release-assets"
            github_root = tmp_path / "github.com" / "ip2a" / "memorph" / "releases" / "download" / f"v{version}"
            install_dir = tmp_path / "bin"

            for platform in platforms:
                platform_dir = dist_root / platform["id"]
                platform_dir.mkdir(parents=True)
                binary = platform_dir / platform["artifact_binary"]
                if platform["id"].startswith("win32-"):
                    binary.write_bytes(b"fake windows binary")
                else:
                    binary.write_text(
                        "#!/usr/bin/env sh\n"
                        f"echo '{platform['artifact_binary']} {version}'\n",
                        encoding="utf-8",
                    )
                    binary.chmod(0o755)
                digest = hashlib.sha256(binary.read_bytes()).hexdigest()
                (platform_dir / "SHA256SUMS").write_text(
                    f"{digest}  {platform['artifact_binary']}\n",
                    encoding="utf-8",
                )

            run_command(
                [
                    "python3",
                    "scripts/prepare_github_release_assets.py",
                    "--dist-root",
                    str(dist_root),
                    "--assets-dir",
                    str(assets_dir),
                ]
            )

            github_root.mkdir(parents=True)
            for asset in assets_dir.iterdir():
                (github_root / asset.name).write_bytes(asset.read_bytes())

            env = os.environ.copy()
            env.update(
                {
                    "VERSION": version,
                    "INSTALL_DIR": str(install_dir),
                    "GITHUB_BASE_URL": f"file://{tmp_path / 'github.com'}",
                }
            )

            result = run_command(["bash", "install.sh"], env=env)
            self.assertIn("[ok] Installed", result.stdout)
            self.assertTrue((install_dir / "memorph").exists())
            self.assertTrue((install_dir / "memo").exists())

            installed_version = subprocess.run(
                [str(install_dir / "memorph"), "--version"],
                check=True,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
            ).stdout.strip()
            self.assertIn(version, installed_version)


if __name__ == "__main__":
    unittest.main(verbosity=2)
