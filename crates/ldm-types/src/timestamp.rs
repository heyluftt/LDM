//! Unix-second timestamps shared by the job model and the desktop app.

use serde::{Deserialize, Deserializer};

/// Current time as Unix seconds; `0` when the clock is before the epoch.
pub fn unix_seconds_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Accepts a JSON number or a decimal string. State files written before timestamps became
/// numbers hold strings such as `"1760000000"`; a string that is not a number maps to `0`
/// ("unknown") rather than rejecting the whole file.
pub fn deserialize_unix_seconds<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum UnixSecondsRepr {
        Number(u64),
        Text(String),
    }

    Ok(match UnixSecondsRepr::deserialize(deserializer)? {
        UnixSecondsRepr::Number(value) => value,
        UnixSecondsRepr::Text(text) => text.trim().parse::<u64>().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Stamped {
        #[serde(deserialize_with = "super::deserialize_unix_seconds")]
        at: u64,
    }

    #[test]
    fn accepts_numbers_and_numeric_strings() {
        let number: Stamped = serde_json::from_str(r#"{"at": 1760000000}"#).unwrap();
        let text: Stamped = serde_json::from_str(r#"{"at": "1760000000"}"#).unwrap();
        assert_eq!(number, Stamped { at: 1_760_000_000 });
        assert_eq!(text, Stamped { at: 1_760_000_000 });
    }

    #[test]
    fn maps_unparsable_strings_to_zero() {
        let value: Stamped = serde_json::from_str(r#"{"at": "soon"}"#).unwrap();
        assert_eq!(value.at, 0);
    }

    #[test]
    fn rejects_other_json_types() {
        assert!(serde_json::from_str::<Stamped>(r#"{"at": null}"#).is_err());
        assert!(serde_json::from_str::<Stamped>(r#"{"at": -5}"#).is_err());
    }

    #[test]
    fn now_is_after_2025() {
        assert!(super::unix_seconds_now() > 1_735_689_600);
    }
}
