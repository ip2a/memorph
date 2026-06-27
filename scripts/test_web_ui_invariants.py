#!/usr/bin/env python3
from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_JS = ROOT / "web" / "app.js"
CHROME_JS = ROOT / "web" / "app" / "chrome.js"
ROUTER_JS = ROOT / "web" / "app" / "router.js"
NAVIGATION_JS = ROOT / "web" / "app" / "navigation.js"
AGENTS_SETTINGS_JS = ROOT / "web" / "app" / "agents_settings.js"
CONSTANTS_JS = ROOT / "web" / "app" / "constants.js"
GITHUB_REPO_URL = "https://github.com/ip2a/memorph"
NPM_PACKAGE_URL = "https://www.npmjs.com/package/memorph"


def read_sources() -> str:
    parts = []
    for path in (APP_JS, CHROME_JS, ROUTER_JS, NAVIGATION_JS, AGENTS_SETTINGS_JS, CONSTANTS_JS):
        if path.exists():
            parts.append(path.read_text(encoding="utf-8"))
    return "\n".join(parts)


class WebUiInvariantTest(unittest.TestCase):
    def test_global_click_handler_intercepts_external_links(self) -> None:
        source = read_sources()
        self.assertIn("const externalLink = event.target.closest('a[href]');", source)
        self.assertIn("if (isExternalHttpUrl(href)) {", source)
        self.assertIn("void openExternalUrl(href);", source)

    def test_provider_toggle_rerenders_agent_filter_modal(self) -> None:
        source = read_sources()
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
        source = read_sources()
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
        source = read_sources()
        self.assertIn(GITHUB_REPO_URL, source)
        self.assertIn('data-action="open-external"', source)
        self.assertIn(NPM_PACKAGE_URL, source)

    def test_topbar_back_uses_route_navigation_metadata(self) -> None:
        source = read_sources()
        self.assertIn('data-action="go-back"', source)
        self.assertIn('class="topbar-back" data-action="go-back"', source)
        self.assertIn("createNavigation", source)
        self.assertIn("fallbackPath: routeBackTarget", source)
        self.assertIn("routeScrollClass(state.route)", source)
        self.assertIn("saveCurrentPageState", source)
        self.assertIn("restorePageState", source)

        chrome = CHROME_JS.read_text(encoding="utf-8")
        brand_cluster = re.search(r'<div class="brand-cluster">(?P<body>.*?)</div>\s*<div class="top-actions">', chrome, re.DOTALL)
        self.assertIsNotNone(brand_cluster)
        self.assertNotIn('data-action="go-back"', brand_cluster.group("body"))
        top_actions = re.search(r'<div class="top-actions">(?P<body>.*?)</div>\s*</nav>', chrome, re.DOTALL)
        self.assertIsNotNone(top_actions)
        top_actions_body = top_actions.group("body").strip()
        self.assertIn('data-action="go-back"', top_actions_body)
        self.assertTrue(top_actions_body.startswith('${state.route.name === "home" ? "" : `<button type="button" class="topbar-back" data-action="go-back"'))
        self.assertNotIn('title="GitHub"', top_actions_body)

    def test_agent_management_loads_summary_before_provider_detail(self) -> None:
        app = APP_JS.read_text(encoding="utf-8")
        summary_match = re.search(
            r"async function loadAgentManagement\(\) \{(?P<body>.*?)\n\}\n\nasync function loadAgentProviderDetail",
            app,
            re.DOTALL,
        )
        self.assertIsNotNone(summary_match)
        summary_body = summary_match.group("body")
        self.assertIn('api("/api/v1/agents/summary")', summary_body)
        self.assertNotIn('api("/api/v1/agents")', summary_body)

        detail_match = re.search(
            r"async function loadAgentProviderDetail\(providerId, options = \{\}\) \{(?P<body>.*?)\n\}\n\nfunction loadSelectedAgentProviderDetail",
            app,
            re.DOTALL,
        )
        self.assertIsNotNone(detail_match)
        self.assertIn('api(`/api/v1/agents/${encodeURIComponent(providerId)}`)', detail_match.group("body"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
