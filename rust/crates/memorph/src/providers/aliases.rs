pub(crate) fn canonical_provider_id(provider_id: &str) -> String {
    match provider_id.trim().to_ascii_lowercase().as_str() {
        "factory" => "droid".to_string(),
        "trae-cn" | "trae_cn" => "traecn".to_string(),
        "trae-gui" => "trae_gui".to_string(),
        "oh-my-pi" | "oh_my_pi" => "omp".to_string(),
        "codybuddy-cn" | "codybuddy_cn" => "codybuddycn".to_string(),
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
        assert_eq!(super::canonical_provider_id("trae-cn"), "traecn");
        assert_eq!(super::canonical_provider_id("trae_cn"), "traecn");
        assert_eq!(super::canonical_provider_id("trae-gui"), "trae_gui");
        assert_eq!(super::canonical_provider_id("oh-my-pi"), "omp");
        assert_eq!(super::canonical_provider_id("oh_my_pi"), "omp");
        assert_eq!(super::canonical_provider_id("codybuddy-cn"), "codybuddycn");
        assert_eq!(super::canonical_provider_id("step-fun"), "stepfun");
        assert_eq!(super::canonical_provider_id("work-buddy"), "workbuddy");
    }
}
