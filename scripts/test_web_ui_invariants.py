#!/usr/bin/env python3
from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_JS = ROOT / "web" / "app.js"
GITHUB_REPO_URL = "https://github.com/ip2a/memorph"
NPM_PACKAGE_URL = "https://www.npmjs.com/package/memorph"


def read_app_js() -> str:
    return APP_JS.read_text(encoding="utf-8")


class WebUiInvariantTest(unittest.TestCase):
    def test_global_click_handler_intercepts_external_links(self) -> None:
        source = read_app_js()
        self.assertIn("const externalLink = event.target.closest('a[href]');", source)
        self.assertIn("if (isExternalHttpUrl(href)) {", source)
        self.assertIn("void openExternalUrl(href);", source)

    def test_provider_toggle_rerenders_agent_filter_modal(self) -> None:
        source = read_app_js()
        match = re.search(
            r"function toggleProvider\(provider, checked\) \{(?P<body>.*?)\n\}",
            source,
            re.DOTALL,
        )
        self.assertIsNotNone(match)
        body = match.group("body")
        self.assertIn('state.modal?.view === "agent-filter"', body)
        self.assertIn("openAgentFilterModal();", body)
        self.assertIn("void persistProvidersAndReload();", body)

    def test_external_opening_prefers_tauri_then_browser_fallback(self) -> None:
        source = read_app_js()
        match = re.search(
            r"async function openExternalUrl\(url\) \{(?P<body>.*?)\n\}",
            source,
            re.DOTALL,
        )
        self.assertIsNotNone(match)
        body = match.group("body")
        self.assertIn("window.__TAURI__?.opener?.openUrl", body)
        self.assertIn('window.open(url, "_blank", "noopener,noreferrer")', body)
        self.assertIn("window.location.href = url;", body)

    def test_known_external_links_use_expected_destinations(self) -> None:
        source = read_app_js()
        self.assertIn(GITHUB_REPO_URL, source)
        self.assertIn(f'data-url="{GITHUB_REPO_URL}"', source)
        self.assertIn('data-action="open-external"', source)
        self.assertIn(NPM_PACKAGE_URL, source)


if __name__ == "__main__":
    unittest.main(verbosity=2)
