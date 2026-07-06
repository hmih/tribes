//! Minimal T3D scalar extractor.
//!
//! The build-time map converter preserves gameplay actor properties as raw T3D
//! text in the `.scene.actors.json` files (e.g. `"TeamIndex=1"`). This module
//! provides typed extraction of *simple scalar* values from those strings —
//! enough for spawn-point team colors, network IDs, and similar.
//!
//! It does NOT parse inline `begin object ... end object` blocks, struct
//! literals like `(A=1,B=2)`, array values, or error markers. That level of
//! parsing is deferred until a later milestone needs weapon/pawn sub-object
//! properties; see PLAN.md "T3D Property Parser".
//!
//! ## Value forms handled
//!
//! | T3D text                 | Extractor        | Result                       |
//! |-------------------------|------------------|------------------------------|
//! | `"TeamIndex=1"`         | [`int`]          | `Some(1)`                    |
//! | `"Priority=2"`          | [`int`]          | `Some(2)`                    |
//! | `"bInitialized=true"`   | [`bool`]         | `Some(true)`                 |
//! | `"GroundSpeed=440.0"`   | [`float`]       | `Some(440.0)`                |
//! | `"Tag=\"PlayerStart\""` | [`string`]       | `Some("PlayerStart")`        |
//! | `"PathList=/* ERROR */"`| any             | `None` (error marker)        |
//!
//! All extractors return `None` if the key is absent, the value is an error
//! marker, or the value cannot be parsed as the requested type.

/// Extracts the raw `Value` portion of a `Key=Value` T3D property string.
///
/// Returns `None` if the key is absent, if the value is an error marker
/// (`/* ERROR: ... */` / `/* Array type was not detected. */`), or if the
/// property is malformed.
fn raw_value<'a>(property_text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = key;
    let rest = property_text.strip_prefix(prefix)?;
    // The char immediately after the key must be '='.
    let rest = rest.strip_prefix('=')?;
    let value = rest.trim();
    // Error markers — the decompiler emits these for unreadable values.
    if value.starts_with("/*") {
        return None;
    }
    Some(value)
}

/// Extracts an `i64` from a `Key=Value` T3D property.
pub fn int(property_text: &str, key: &str) -> Option<i64> {
    raw_value(property_text, key)?.parse().ok()
}

/// Extracts an `f32` from a `Key=Value` T3D property.
pub fn float(property_text: &str, key: &str) -> Option<f32> {
    raw_value(property_text, key)?.parse().ok()
}

/// Extracts a `bool` from a `Key=Value` T3D property.
///
/// T3D uses `true`/`false` (lowercase), as seen in properties like
/// `bInitialized=true`. Also accepts `1`/`0` for robustness.
pub fn bool(property_text: &str, key: &str) -> Option<bool> {
    let v = raw_value(property_text, key)?;
    match v {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Extracts a quoted string from a `Key="Value"` T3D property.
///
/// T3D string values are double-quoted (e.g. `Tag="PlayerStart"`). Returns the
/// unquoted contents. Returns `None` if the value is not quoted.
pub fn string<'a>(property_text: &'a str, key: &str) -> Option<&'a str> {
    let v = raw_value(property_text, key)?;
    let v = v.strip_prefix('"')?;
    let v = v.strip_suffix('"')?;
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_present() {
        assert_eq!(int("TeamIndex=1", "TeamIndex"), Some(1));
        assert_eq!(int("NetworkID=4", "NetworkID"), Some(4));
        assert_eq!(int("Priority=2", "Priority"), Some(2));
    }

    #[test]
    fn int_absent_key() {
        assert_eq!(int("TeamIndex=1", "TeamNumber"), None);
    }

    #[test]
    fn int_error_marker() {
        assert_eq!(
            int(
                "PathList=/* ERROR: ArgumentOutOfRangeException */",
                "PathList"
            ),
            None
        );
        assert_eq!(
            int(
                "ShootSpots=/* Array type was not detected. */",
                "ShootSpots"
            ),
            None
        );
    }

    #[test]
    fn float_present() {
        assert_eq!(float("GroundSpeed=440.0", "GroundSpeed"), Some(440.0));
        assert_eq!(
            float("visitedWeight=9999840", "visitedWeight"),
            Some(9999840.0)
        );
    }

    #[test]
    fn bool_present() {
        assert_eq!(bool("bInitialized=true", "bInitialized"), Some(true));
        assert_eq!(bool("bCanSki=true", "bCanSki"), Some(true));
        assert_eq!(bool("bCanSki=false", "bCanSki"), Some(false));
    }

    #[test]
    fn string_present() {
        assert_eq!(string("Tag=\"PlayerStart\"", "Tag"), Some("PlayerStart"));
        assert_eq!(
            string("Tag=\"TrInventoryStation_BloodEagle\"", "Tag"),
            Some("TrInventoryStation_BloodEagle"),
        );
    }

    #[test]
    fn string_unquoted_is_none() {
        // Bare enum-style values like `m_ContextLocation=EVGSContextLocation.VGSContext_NearFlag`
        // are NOT quoted strings; the caller should treat them as raw via
        // `raw_value`-style helpers, not `string`.
        assert_eq!(
            string(
                "m_ContextLocation=EVGSContextLocation.VGSContext_NearFlag",
                "m_ContextLocation"
            ),
            None
        );
    }

    #[test]
    fn struct_literal_is_not_scalar() {
        // Struct literals like `MaxPathSize=(Radius=140.0,Height=100.0)` are
        // intentionally not parseable by the scalar helpers.
        let s = "MaxPathSize=(Radius=140.0000000,Height=100.0000000)";
        assert_eq!(int(s, "MaxPathSize"), None);
        assert_eq!(float(s, "MaxPathSize"), None);
        assert_eq!(string(s, "MaxPathSize"), None);
    }

    #[test]
    fn inline_subobject_not_parseable() {
        // Inline `begin object ... end object` blocks are intentionally not
        // parseable by the scalar helpers; deferred to the full T3D parser.
        let s = "ArmsMesh=UDKSkeletalMeshComponent_6\r\nbegin object name=\"X\"\r\nend object";
        assert_eq!(string(s, "ArmsMesh"), None);
    }
}
