use anyhow::Result;
use serde_json::Value;

pub(crate) trait ProviderSettingCompat: Send + Sync {
    fn provider_id(&self) -> &'static str;

    fn legacy_web_preference_default(&self, _key: &str) -> Option<Value> {
        None
    }

    fn apply_legacy_web_preference(
        &self,
        _prefs: &mut crate::config::WebPreferences,
        _key: &str,
        _value: &Value,
    ) -> Result<bool> {
        Ok(false)
    }

    fn sync_legacy_field_from_provider_preference(
        &self,
        _prefs: &mut crate::config::WebPreferences,
        _key: &str,
        _value: Option<&Value>,
    ) -> bool {
        false
    }

    fn hydrate_legacy_preferences(&self, _prefs: &mut crate::config::WebPreferences) {}
}

static OPENCODE_COMPAT: super::opencode::settings_compat::OpenCodeSettingCompat =
    super::opencode::settings_compat::OpenCodeSettingCompat;

const PROVIDER_SETTING_COMPATS: &[&dyn ProviderSettingCompat] = &[&OPENCODE_COMPAT];

pub(crate) fn legacy_web_preference_default_bool(key: &str) -> Option<bool> {
    PROVIDER_SETTING_COMPATS
        .iter()
        .find_map(|compat| compat.legacy_web_preference_default(key))
        .and_then(|value| value.as_bool())
}

pub(crate) fn apply_legacy_web_preference(
    prefs: &mut crate::config::WebPreferences,
    key: &str,
    value: &Value,
) -> Result<bool> {
    for compat in PROVIDER_SETTING_COMPATS {
        if compat.apply_legacy_web_preference(prefs, key, value)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn sync_legacy_field_from_provider_preference(
    prefs: &mut crate::config::WebPreferences,
    provider_id: &str,
    key: &str,
    value: Option<&Value>,
) {
    let Some(compat) = PROVIDER_SETTING_COMPATS
        .iter()
        .find(|compat| compat.provider_id() == provider_id)
    else {
        return;
    };

    compat.sync_legacy_field_from_provider_preference(prefs, key, value);
}

pub(crate) fn hydrate_legacy_preferences(prefs: &mut crate::config::WebPreferences) {
    for compat in PROVIDER_SETTING_COMPATS {
        compat.hydrate_legacy_preferences(prefs);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn registry_exposes_legacy_default_for_opencode_toggle() {
        assert_eq!(
            super::legacy_web_preference_default_bool("show_opencode_subagents"),
            Some(false)
        );
    }

    #[test]
    fn registry_routes_legacy_update_to_provider_setting() {
        let mut prefs = crate::config::WebPreferences::default();

        assert!(super::apply_legacy_web_preference(
            &mut prefs,
            "show_opencode_subagents",
            &Value::Bool(true),
        )
        .unwrap());

        assert_eq!(
            crate::config::provider_preference_from_prefs(&prefs, "opencode", "show_subagents")
                .and_then(Value::as_bool),
            Some(true)
        );
    }
}
