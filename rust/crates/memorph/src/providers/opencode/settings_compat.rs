use anyhow::Result;
use serde_json::Value;

const PROVIDER_ID: &str = "opencode";
const SETTING_ID: &str = "show_subagents";
const LEGACY_WEB_KEY: &str = "show_opencode_subagents";
const DEFAULT_SHOW_SUBAGENTS: bool = false;

pub(crate) struct OpenCodeSettingCompat;

impl OpenCodeSettingCompat {
    fn default_show_subagents(&self) -> bool {
        DEFAULT_SHOW_SUBAGENTS
    }
}

impl crate::providers::setting_compat_registry::ProviderSettingCompat for OpenCodeSettingCompat {
    fn provider_id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn legacy_web_preference_default(&self, key: &str) -> Option<Value> {
        (key == LEGACY_WEB_KEY).then_some(Value::Bool(self.default_show_subagents()))
    }

    fn apply_legacy_web_preference(
        &self,
        prefs: &mut crate::config::WebPreferences,
        key: &str,
        value: &Value,
    ) -> Result<bool> {
        if key != LEGACY_WEB_KEY {
            return Ok(false);
        }

        let Some(value) = value.as_bool() else {
            anyhow::bail!(
                "Legacy web preference expects boolean value: {}",
                LEGACY_WEB_KEY
            );
        };

        prefs.show_opencode_subagents = value;
        crate::config::set_provider_preference_in_prefs(
            prefs,
            PROVIDER_ID,
            SETTING_ID,
            Some(Value::Bool(value)),
        )?;

        Ok(true)
    }

    fn sync_legacy_field_from_provider_preference(
        &self,
        prefs: &mut crate::config::WebPreferences,
        key: &str,
        value: Option<&Value>,
    ) -> bool {
        if key != SETTING_ID {
            return false;
        }

        prefs.show_opencode_subagents = value
            .and_then(Value::as_bool)
            .unwrap_or_else(|| self.default_show_subagents());
        true
    }

    fn hydrate_legacy_preferences(&self, prefs: &mut crate::config::WebPreferences) {
        if crate::config::provider_preference_from_prefs(prefs, PROVIDER_ID, SETTING_ID).is_none() {
            let _ = crate::config::set_provider_preference_in_prefs(
                prefs,
                PROVIDER_ID,
                SETTING_ID,
                Some(Value::Bool(prefs.show_opencode_subagents)),
            );
        }

        prefs.show_opencode_subagents =
            crate::config::provider_preference_from_prefs(prefs, PROVIDER_ID, SETTING_ID)
                .and_then(Value::as_bool)
                .unwrap_or_else(|| self.default_show_subagents());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn hydrate_backfills_provider_preference_from_legacy_field() {
        let mut prefs = crate::config::WebPreferences::default();
        prefs.show_opencode_subagents = true;

        <super::OpenCodeSettingCompat as crate::providers::setting_compat_registry::ProviderSettingCompat>::hydrate_legacy_preferences(
            &super::OpenCodeSettingCompat,
            &mut prefs,
        );

        assert_eq!(
            crate::config::provider_preference_from_prefs(&prefs, "opencode", "show_subagents")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn provider_preference_sync_updates_legacy_field() {
        let mut prefs = crate::config::WebPreferences::default();

        <super::OpenCodeSettingCompat as crate::providers::setting_compat_registry::ProviderSettingCompat>::sync_legacy_field_from_provider_preference(
            &super::OpenCodeSettingCompat,
            &mut prefs,
            "show_subagents",
            Some(&Value::Bool(true)),
        );
        assert!(prefs.show_opencode_subagents);

        <super::OpenCodeSettingCompat as crate::providers::setting_compat_registry::ProviderSettingCompat>::sync_legacy_field_from_provider_preference(
            &super::OpenCodeSettingCompat,
            &mut prefs,
            "show_subagents",
            None,
        );
        assert!(!prefs.show_opencode_subagents);
    }
}
