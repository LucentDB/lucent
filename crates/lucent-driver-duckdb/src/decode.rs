//! Pure `ValueRef` → `Value` decoding.
//!
//! No I/O, no async — every test here runs in microseconds without a database,
//! matching the Postgres decoder's contract from Plan A.
//!
//! The rules that matter:
//! - Anything wider than `i64` becomes `Decimal(String)`. Truncating to fit is
//!   data corruption, not degradation.
//! - Timezone-awareness comes from the column's **declared type**, because
//!   `ValueRef::Timestamp` carries no zone flag.
//! - Anything unmapped becomes `Other { type_name, text }`, never an error.

use duckdb::types::{TimeUnit, ValueRef};
use lucent_protocol::Value;

const MICROS_PER_SEC: i64 = 1_000_000;

/// True when a column's declared type stores instants rather than wall-clock
/// readings.
pub fn is_tz_aware(decl_type: &str) -> bool {
    let upper = decl_type.to_ascii_uppercase();
    upper == "TIMESTAMPTZ" || upper.contains("WITH TIME ZONE")
}

/// Scale a unit-tagged integer to microseconds. Sub-microsecond precision is
/// truncated toward negative infinity, matching the protocol's fixed micros
/// resolution.
fn to_micros(unit: TimeUnit, value: i64) -> i64 {
    match unit {
        TimeUnit::Second => value.saturating_mul(MICROS_PER_SEC),
        TimeUnit::Millisecond => value.saturating_mul(1_000),
        TimeUnit::Microsecond => value,
        TimeUnit::Nanosecond => value.div_euclid(1_000),
    }
}

fn other(decl_type: &str, text: impl Into<String>) -> Value {
    Value::Other {
        type_name: decl_type.to_string(),
        text: text.into(),
    }
}

/// Types whose text rendering hides a structure: lists, structs, maps, unions.
fn is_composite(decl_type: &str) -> bool {
    let upper = decl_type.to_ascii_uppercase();
    upper.ends_with("[]")
        || upper.starts_with("STRUCT")
        || upper.starts_with("MAP")
        || upper.starts_with("UNION")
}

/// Render a DuckDB interval in a readable, stable form.
///
/// DuckDB stores months, days, and nanoseconds independently — a month is not
/// a fixed number of days, so they cannot be collapsed. The time part always
/// renders, so a zero interval is `00:00:00` rather than an empty string that
/// would read as NULL in the grid.
///
/// Sign rule: a time-only interval carries its sign on the time part
/// (`-00:00:02`). When months or days are present they carry the interval's
/// sign, and the time part renders as a positive magnitude
/// (`-1 months -2 days 03:00:00`).
fn format_interval(months: i32, days: i32, nanos: i64) -> String {
    let mut parts = Vec::new();
    if months != 0 {
        parts.push(format!("{months} months"));
    }
    if days != 0 {
        parts.push(format!("{days} days"));
    }
    // Work with the unsigned magnitude so the sign is decided once, up front.
    // Truncating division toward zero would silently drop the sign of any
    // sub-hour interval (-2s must not render as "00:00:02").
    let negative = nanos < 0;
    let magnitude = nanos.unsigned_abs();
    let total_secs = (magnitude / 1_000_000_000) as i64;
    let sub_micros = ((magnitude % 1_000_000_000) / 1_000) as i64;
    let (h, m, s) = (total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60);
    let time = if sub_micros == 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}.{sub_micros:06}")
    };
    let time = if negative && months == 0 && days == 0 {
        format!("-{time}")
    } else {
        time
    };
    parts.push(time);
    parts.join(" ")
}

/// Decode one cell against its column's declared type.
pub fn duck_value_to_value(value: ValueRef<'_>, decl_type: &str) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Boolean(b) => Value::Bool(b),

        ValueRef::TinyInt(v) => Value::Int64(v as i64),
        ValueRef::SmallInt(v) => Value::Int64(v as i64),
        ValueRef::Int(v) => Value::Int64(v as i64),
        ValueRef::BigInt(v) => Value::Int64(v),
        ValueRef::UTinyInt(v) => Value::Int64(v as i64),
        ValueRef::USmallInt(v) => Value::Int64(v as i64),
        ValueRef::UInt(v) => Value::Int64(v as i64),

        // Wider than i64: exact text, never a truncated integer.
        ValueRef::UBigInt(v) => match i64::try_from(v) {
            Ok(n) => Value::Int64(n),
            Err(_) => Value::Decimal(v.to_string()),
        },
        ValueRef::HugeInt(v) => Value::Decimal(v.to_string()),
        ValueRef::UHugeInt(v) => Value::Decimal(v.to_string()),

        ValueRef::Float(f) => Value::Float64(f as f64),
        ValueRef::Double(f) => Value::Float64(f),
        ValueRef::Decimal(d) => Value::Decimal(d.to_string()),

        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(s) if decl_type.eq_ignore_ascii_case("JSON") => Value::Json(s.to_string()),
            // A composite type rendered as text is still a composite type; tag
            // it so downstream sees the shape, not an anonymous string.
            Ok(s) if is_composite(decl_type) => other(decl_type, s),
            Ok(s) => Value::Text(s.to_string()),
            // Invalid UTF-8 must not error the query.
            Err(_) => other(decl_type, String::from_utf8_lossy(bytes)),
        },
        ValueRef::Blob(bytes) => Value::Binary(bytes.to_vec()),

        ValueRef::Date32(days) => Value::Date(days),
        ValueRef::Time64(unit, v) => Value::Time(to_micros(unit, v)),
        ValueRef::Timestamp(unit, v) => Value::Timestamp {
            micros: to_micros(unit, v),
            tz: is_tz_aware(decl_type),
        },
        ValueRef::Interval {
            months,
            days,
            nanos,
        } => Value::Interval(format_interval(months, days, nanos)),

        // Lists, structs, maps, arrays, unions, enums, geometry. Tagged with
        // the source type so the grid and the AI both know what they are
        // looking at, rather than seeing anonymous text.
        other_value => other(decl_type, format!("{other_value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use duckdb::types::{TimeUnit, ValueRef};
    use lucent_protocol::Value;

    use super::{duck_value_to_value, is_tz_aware};

    fn d(value: ValueRef<'_>, decl: &str) -> Value {
        duck_value_to_value(value, decl)
    }

    #[test]
    fn decodes_every_signed_and_small_unsigned_integer_as_int64() {
        assert!(matches!(
            d(ValueRef::TinyInt(-8), "TINYINT"),
            Value::Int64(-8)
        ));
        assert!(matches!(
            d(ValueRef::SmallInt(16), "SMALLINT"),
            Value::Int64(16)
        ));
        assert!(matches!(d(ValueRef::Int(32), "INTEGER"), Value::Int64(32)));
        assert!(matches!(
            d(ValueRef::BigInt(64), "BIGINT"),
            Value::Int64(64)
        ));
        assert!(matches!(
            d(ValueRef::UTinyInt(8), "UTINYINT"),
            Value::Int64(8)
        ));
        assert!(matches!(
            d(ValueRef::USmallInt(16), "USMALLINT"),
            Value::Int64(16)
        ));
        assert!(matches!(
            d(ValueRef::UInt(32), "UINTEGER"),
            Value::Int64(32)
        ));
    }

    #[test]
    fn integers_too_wide_for_i64_become_exact_decimals_never_truncated_ints() {
        // Truncating a 128-bit integer to fit is data corruption. The exact
        // text is the only lossless representation the protocol has.
        match d(ValueRef::HugeInt(i128::MAX), "HUGEINT") {
            Value::Decimal(s) => assert_eq!(s, i128::MAX.to_string()),
            other => panic!("HUGEINT must never truncate: {other:?}"),
        }
        match d(ValueRef::UBigInt(u64::MAX), "UBIGINT") {
            Value::Decimal(s) => assert_eq!(s, u64::MAX.to_string()),
            other => panic!("an out-of-range UBIGINT must not truncate: {other:?}"),
        }
        // In-range unsigned values are ordinary integers.
        assert!(matches!(
            d(ValueRef::UBigInt(7), "UBIGINT"),
            Value::Int64(7)
        ));
    }

    #[test]
    fn decodes_floats_and_booleans() {
        match d(ValueRef::Double(1.5), "DOUBLE") {
            Value::Float64(f) => assert_eq!(f, 1.5),
            other => panic!("{other:?}"),
        }
        match d(ValueRef::Float(0.5), "FLOAT") {
            Value::Float64(f) => assert_eq!(f, 0.5),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            d(ValueRef::Boolean(true), "BOOLEAN"),
            Value::Bool(true)
        ));
        assert!(matches!(d(ValueRef::Null, "INTEGER"), Value::Null));
    }

    #[test]
    fn text_and_blobs_keep_their_bytes() {
        assert!(matches!(d(ValueRef::Text(b"hello"), "VARCHAR"), Value::Text(s) if s == "hello"));
        assert!(
            matches!(d(ValueRef::Blob(&[0xde, 0xad]), "BLOB"), Value::Binary(b) if b == vec![0xde, 0xad])
        );
        // Invalid UTF-8 must not error the query.
        assert!(matches!(
            d(ValueRef::Text(&[0xff, 0xfe]), "VARCHAR"),
            Value::Other { .. }
        ));
    }

    #[test]
    fn json_columns_are_tagged_as_json_not_plain_text() {
        assert!(
            matches!(d(ValueRef::Text(br#"{"a":1}"#), "JSON"), Value::Json(s) if s == r#"{"a":1}"#)
        );
        // The declared-type match is case-insensitive — DuckDB emits "json"
        // as often as "JSON".
        assert!(
            matches!(d(ValueRef::Text(br#"{"a":1}"#), "json"), Value::Json(s) if s == r#"{"a":1}"#)
        );
    }

    #[test]
    fn dates_are_days_since_the_unix_epoch() {
        assert!(matches!(d(ValueRef::Date32(0), "DATE"), Value::Date(0)));
        assert!(matches!(
            d(ValueRef::Date32(19737), "DATE"),
            Value::Date(19737)
        ));
        assert!(matches!(d(ValueRef::Date32(-1), "DATE"), Value::Date(-1)));
    }

    #[test]
    fn every_time_unit_converts_to_micros() {
        // The protocol's Time is micros since midnight. DuckDB may hand back
        // any of four units, and getting the scaling wrong shifts every
        // displayed time silently.
        assert!(matches!(
            d(ValueRef::Time64(TimeUnit::Second, 1), "TIME"),
            Value::Time(1_000_000)
        ));
        assert!(matches!(
            d(ValueRef::Time64(TimeUnit::Millisecond, 1), "TIME"),
            Value::Time(1_000)
        ));
        assert!(matches!(
            d(ValueRef::Time64(TimeUnit::Microsecond, 1), "TIME"),
            Value::Time(1)
        ));
        // Sub-microsecond precision is truncated, not rounded — matching how
        // the protocol's fixed micros resolution behaves everywhere else.
        assert!(matches!(
            d(ValueRef::Time64(TimeUnit::Nanosecond, 1_500), "TIME"),
            Value::Time(1)
        ));
    }

    #[test]
    fn timezone_awareness_comes_from_the_declared_type_not_the_value() {
        // ValueRef::Timestamp carries no zone flag, so the column's declared
        // type is the only signal. Getting this wrong mislabels instants as
        // wall-clock readings and shifts them on display.
        assert!(is_tz_aware("TIMESTAMP WITH TIME ZONE"));
        assert!(is_tz_aware("TIMESTAMPTZ"));
        assert!(is_tz_aware("timestamptz"));
        assert!(!is_tz_aware("TIMESTAMP"));
        assert!(!is_tz_aware("TIMESTAMP_NS"));
        assert!(!is_tz_aware("DATE"));

        match d(ValueRef::Timestamp(TimeUnit::Microsecond, 0), "TIMESTAMPTZ") {
            Value::Timestamp { micros, tz } => {
                assert_eq!(micros, 0);
                assert!(tz, "a TIMESTAMPTZ column is an instant");
            }
            other => panic!("{other:?}"),
        }
        match d(
            ValueRef::Timestamp(TimeUnit::Millisecond, 1_000),
            "TIMESTAMP",
        ) {
            Value::Timestamp { micros, tz } => {
                assert_eq!(micros, 1_000_000, "milliseconds must scale to micros");
                assert!(!tz, "a bare TIMESTAMP is a wall-clock reading");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn intervals_render_in_a_stable_readable_form() {
        let v = d(
            ValueRef::Interval {
                months: 14,
                days: 3,
                nanos: 3_600_000_000_000,
            },
            "INTERVAL",
        );
        match v {
            Value::Interval(s) => {
                assert!(s.contains("14 months"), "{s}");
                assert!(s.contains("3 days"), "{s}");
                assert!(s.contains("01:00:00"), "{s}");
            }
            other => panic!("{other:?}"),
        }
        // A zero interval must still render as something, not as an empty
        // string that looks like a NULL in the grid.
        match d(
            ValueRef::Interval {
                months: 0,
                days: 0,
                nanos: 0,
            },
            "INTERVAL",
        ) {
            Value::Interval(s) => assert_eq!(s, "00:00:00"),
            other => panic!("{other:?}"),
        }
        // Negative intervals must keep their sign — date arithmetic makes
        // them routine, and a sub-hour one previously lost its sign entirely.
        match d(
            ValueRef::Interval {
                months: 0,
                days: 0,
                nanos: -1_500_000_000,
            },
            "INTERVAL",
        ) {
            Value::Interval(s) => assert_eq!(s, "-00:00:01.500000"),
            other => panic!("{other:?}"),
        }
        match d(
            ValueRef::Interval {
                months: 0,
                days: 0,
                nanos: -2_000_000_000,
            },
            "INTERVAL",
        ) {
            Value::Interval(s) => assert_eq!(s, "-00:00:02"),
            other => panic!("{other:?}"),
        }
        // A whole negative hour keeps its sign through the hours field.
        match d(
            ValueRef::Interval {
                months: 0,
                days: 0,
                nanos: -3_600_000_000_000,
            },
            "INTERVAL",
        ) {
            Value::Interval(s) => assert_eq!(s, "-01:00:00"),
            other => panic!("{other:?}"),
        }
        // With months/days present they carry the sign; the time part renders
        // as a positive magnitude so the parts read left to right.
        match d(
            ValueRef::Interval {
                months: -1,
                days: -2,
                nanos: 3 * 3_600_000_000_000,
            },
            "INTERVAL",
        ) {
            Value::Interval(s) => assert_eq!(s, "-1 months -2 days 03:00:00"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nested_and_unmapped_types_take_the_escape_hatch_with_their_type_name() {
        match d(ValueRef::Null, "STRUCT(a INTEGER)") {
            // NULL wins over the escape hatch — a NULL struct is still NULL.
            Value::Null => {}
            other => panic!("{other:?}"),
        }
        match d(ValueRef::Text(b"[1, 2, 3]"), "INTEGER[]") {
            Value::Other { type_name, text } => {
                assert_eq!(type_name, "INTEGER[]");
                assert_eq!(text, "[1, 2, 3]");
            }
            other => panic!("a list must be tagged, not silently textified: {other:?}"),
        }
    }
}
