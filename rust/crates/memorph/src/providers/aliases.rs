pub(crate) fn canonical_provider_id(provider_id: &str) -> String {
    match provider_id.trim().to_ascii_lowercase().as_str() {
        "factory" => "droid".to_string(),
        "step-fun" | "step_fun" => "stepfun".to_string(),
        "work-buddy" | "work_buddy" => "workbuddy".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonicalizes_known_provider_aliases() {
        assert_eq!(super::canonical_provider_id("factory"), "droid");
        assert_eq!(super::canonical_provider_id("step-fun"), "stepfun");
        assert_eq!(super::canonical_provider_id("work-buddy"), "workbuddy");
    }
}
