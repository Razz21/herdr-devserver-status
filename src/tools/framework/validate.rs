//! Per-file spec validation. Pure function, no file I/O — takes raw YAML
//! text in, returns either an accepted `FrameworkSpec` or the reason it was
//! rejected. Unit-tested directly against string fixtures.

use regex::Regex;

use super::spec::{FrameworkSpec, YamlFrameworkSpec, SUPPORTED_SCHEMA_VERSION};

pub fn validate(raw_yaml: &str) -> Result<FrameworkSpec, String> {
    // Check 1: strict schema. `#[serde(deny_unknown_fields)]` on
    // `YamlFrameworkSpec`/`YamlStateLabels` makes an unknown key a parse
    // error here too, not just a missing/mistyped one.
    let yaml: YamlFrameworkSpec =
        serde_norway::from_str(raw_yaml).map_err(|e| format!("schema error: {e}"))?;

    // Check 2: schema_version this loader understands.
    if yaml.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version: {}",
            yaml.schema_version
        ));
    }

    // Check 3: required fields non-empty after trim.
    let required: [(&str, &str); 5] = [
        ("source", &yaml.source),
        ("agent_name", &yaml.agent_name),
        ("display_agent", &yaml.display_agent),
        ("package_name", &yaml.package_name),
        ("bin_path_pattern", &yaml.bin_path_pattern),
    ];
    for (field, value) in required {
        if value.trim().is_empty() {
            return Err(format!("empty required field: {field}"));
        }
    }

    // Check 4: bin_path_pattern compiles.
    let bin_re = Regex::new(&yaml.bin_path_pattern)
        .map_err(|e| format!("invalid bin_path_pattern: {e}"))?;

    // Check 5: bin_path_pattern must not match the empty string — guards
    // against overly broad patterns like `.*` matching every process. A
    // heuristic floor, not a full guarantee (a pattern can still be broad
    // without matching ""); the real backstop is `find_owning_package`
    // still requiring a real package.json with a matching `name`.
    if bin_re.is_match("") {
        return Err("bin_path_pattern too broad (matches empty string)".to_owned());
    }

    // Check 6: url_pattern, if present, compiles and has exactly 2 capture
    // groups (full URL, port).
    if let Some(pattern) = &yaml.url_pattern {
        let url_re = Regex::new(pattern).map_err(|e| format!("invalid url_pattern: {e}"))?;
        // captures_len() counts group 0 (the whole match) too.
        let group_count = url_re.captures_len() - 1;
        if group_count != 2 {
            return Err(format!(
                "invalid url_pattern: expected 2 capture groups, got {group_count}"
            ));
        }
    }

    // Check 7: signals non-empty, every needle non-empty after trim.
    if yaml.signals.is_empty() {
        return Err("signals empty or contains empty needle".to_owned());
    }
    if yaml.signals.iter().any(|s| s.needle.trim().is_empty()) {
        return Err("signals empty or contains empty needle".to_owned());
    }

    Ok(FrameworkSpec::from(yaml))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_yaml() -> String {
        r#"
schema_version: 1
source: "custom:vite"
agent_name: "vite"
display_agent: "Vite (dev server)"
starting_message: "Vite dev server starting"
state_labels:
  idle: "serving"
  working: "building"
  blocked: "build error"
clear_token_names:
  - "vite_has_errors"
bin_path_pattern: '(^|/)vite/bin/vite\.js$'
package_name: "vite"
signals:
  - kind: starting
    needle: "VITE v"
  - kind: ready
    needle: "ready in"
    counts_as_recent_success: true
url_pattern: 'Local:\s+(https?://[^\s:]+:(\d+)/?)'
"#
        .to_owned()
    }

    #[test]
    fn accepts_valid_spec() {
        let spec = validate(&valid_yaml()).expect("should validate");
        assert_eq!(spec.agent_name, "vite");
        assert_eq!(
            spec.state_labels,
            vec![
                ("idle".to_owned(), "serving".to_owned()),
                ("working".to_owned(), "building".to_owned()),
                ("blocked".to_owned(), "build error".to_owned()),
            ]
        );
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = valid_yaml().replace(
            "package_name: \"vite\"",
            "package_name: \"vite\"\nbogus_field: \"x\"",
        );
        let err = validate(&yaml).unwrap_err();
        assert!(err.starts_with("schema error:"), "got: {err}");
    }

    #[test]
    fn rejects_missing_required_field() {
        let yaml = valid_yaml().replace("package_name: \"vite\"\n", "");
        let err = validate(&yaml).unwrap_err();
        assert!(err.starts_with("schema error:"), "got: {err}");
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let yaml = valid_yaml().replace("schema_version: 1", "schema_version: 2");
        assert_eq!(
            validate(&yaml).unwrap_err(),
            "unsupported schema_version: 2"
        );
    }

    #[test]
    fn rejects_empty_required_field() {
        let yaml = valid_yaml().replace("agent_name: \"vite\"", "agent_name: \"   \"");
        assert_eq!(validate(&yaml).unwrap_err(), "empty required field: agent_name");
    }

    #[test]
    fn rejects_invalid_bin_path_pattern() {
        let yaml = valid_yaml().replace(
            "bin_path_pattern: '(^|/)vite/bin/vite\\.js$'",
            "bin_path_pattern: '(unclosed'",
        );
        let err = validate(&yaml).unwrap_err();
        assert!(err.starts_with("invalid bin_path_pattern:"), "got: {err}");
    }

    #[test]
    fn rejects_bin_path_pattern_matching_empty_string() {
        let yaml = valid_yaml().replace(
            "bin_path_pattern: '(^|/)vite/bin/vite\\.js$'",
            "bin_path_pattern: '.*'",
        );
        assert_eq!(
            validate(&yaml).unwrap_err(),
            "bin_path_pattern too broad (matches empty string)"
        );
    }

    #[test]
    fn rejects_url_pattern_with_wrong_group_count() {
        let yaml = valid_yaml().replace(
            "url_pattern: 'Local:\\s+(https?://[^\\s:]+:(\\d+)/?)'",
            "url_pattern: 'Local:\\s+(https?://[^\\s:]+:\\d+/?)'",
        );
        let err = validate(&yaml).unwrap_err();
        assert!(
            err.starts_with("invalid url_pattern: expected 2 capture groups"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_empty_signals() {
        let yaml = valid_yaml().replace(
            "signals:\n  - kind: starting\n    needle: \"VITE v\"\n  - kind: ready\n    needle: \"ready in\"\n    counts_as_recent_success: true\n",
            "signals: []\n",
        );
        assert_eq!(
            validate(&yaml).unwrap_err(),
            "signals empty or contains empty needle"
        );
    }

    #[test]
    fn rejects_empty_needle() {
        let yaml = valid_yaml().replace("needle: \"VITE v\"", "needle: \"\"");
        assert_eq!(
            validate(&yaml).unwrap_err(),
            "signals empty or contains empty needle"
        );
    }

    #[test]
    fn url_pattern_is_optional() {
        let yaml = valid_yaml()
            .lines()
            .filter(|l| !l.starts_with("url_pattern"))
            .collect::<Vec<_>>()
            .join("\n");
        let spec = validate(&yaml).expect("should validate without url_pattern");
        assert!(spec.url_pattern.is_none());
    }
}
