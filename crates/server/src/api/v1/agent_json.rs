use serde::de::DeserializeOwned;

pub(crate) const AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES: usize = 16 * 1024;
pub(crate) const AGENT_TELEMETRY_JSON_MAX_BYTES: usize = 256 * 1024;

pub(crate) fn parse_dashboard_metadata_json<T>(value: Option<&str>) -> Option<T>
where
    T: DeserializeOwned,
{
    parse_bounded_agent_json(value, AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES)
}

pub(crate) fn parse_agent_telemetry_json<T>(value: Option<&str>) -> Option<T>
where
    T: DeserializeOwned,
{
    parse_bounded_agent_json(value, AGENT_TELEMETRY_JSON_MAX_BYTES)
}

fn parse_bounded_agent_json<T>(value: Option<&str>, max_bytes: usize) -> Option<T>
where
    T: DeserializeOwned,
{
    let value = value?;
    if value.len() > max_bytes {
        return None;
    }
    let value = value.trim();
    if value.is_empty() || value.len() > max_bytes {
        return None;
    }
    serde_json::from_str(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn dashboard_metadata_json_is_bounded_before_parse() {
        let value = format!(
            r#"{{"tags":["{}"]}}"#,
            "x".repeat(AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES)
        );

        assert!(parse_dashboard_metadata_json::<Value>(Some(&value)).is_none());
        assert!(parse_dashboard_metadata_json::<Value>(Some(r#"{"tags":["prod"]}"#)).is_some());
    }

    #[test]
    fn dashboard_metadata_json_counts_outer_whitespace_in_budget() {
        let value = format!(
            "{}{{\"tags\":[\"prod\"]}}",
            " ".repeat(AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES)
        );

        assert!(parse_dashboard_metadata_json::<Value>(Some(&value)).is_none());
    }

    #[test]
    fn telemetry_json_is_bounded_before_parse() {
        let value = format!(
            r#"{{"message":"{}"}}"#,
            "x".repeat(AGENT_TELEMETRY_JSON_MAX_BYTES)
        );

        assert!(parse_agent_telemetry_json::<Value>(Some(&value)).is_none());
        assert!(parse_agent_telemetry_json::<Value>(Some(r#"{"cpu_percent":42}"#)).is_some());
    }
}
