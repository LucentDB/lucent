//! Pure text-to-`Value` decoding for the Postgres driver.
//!
//! The connector calls `prepare()` for column metadata and `simple_query_raw`
//! for values, so every value arrives as text that PostgreSQL itself rendered,
//! alongside the column's type OID. Decoding is therefore a pure parse keyed on
//! that OID — no binary decoders, no protocol change.
//!
//! Every parse failure degrades to `Value::Other`. A display path must never
//! turn a surprising value into a failed query.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use lucent_protocol::Value;
use tokio_postgres::types::Type;

/// Micros in one second.
const MICROS_PER_SEC: i64 = 1_000_000;

/// Fall back to the tagged-text escape hatch.
fn other(ty: &Type, text: &str) -> Value {
    Value::Other {
        type_name: ty.name().to_string(),
        text: text.to_string(),
    }
}

/// Decode one text cell against its column type.
pub fn pg_text_to_value(text: &str, ty: &Type) -> Value {
    match *ty {
        Type::BOOL => match text {
            "t" => Value::Bool(true),
            "f" => Value::Bool(false),
            _ => other(ty, text),
        },

        Type::INT2 | Type::INT4 | Type::INT8 => text
            .parse::<i64>()
            .map(Value::Int64)
            .unwrap_or_else(|_| other(ty, text)),

        // `parse::<f64>` already accepts "NaN", "inf", "infinity" case-insensitively,
        // which covers exactly what Postgres emits for float4/float8.
        Type::FLOAT4 | Type::FLOAT8 => text
            .parse::<f64>()
            .map(Value::Float64)
            .unwrap_or_else(|_| other(ty, text)),

        // Verbatim. Parsing this into any fixed-point or float type loses data.
        Type::NUMERIC => Value::Decimal(text.to_string()),

        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => Value::Text(text.to_string()),

        Type::BYTEA => decode_bytea_hex(text)
            .map(Value::Binary)
            .unwrap_or_else(|| other(ty, text)),

        Type::JSON | Type::JSONB => Value::Json(text.to_string()),

        Type::UUID => text
            .parse::<uuid::Uuid>()
            .map(Value::Uuid)
            .unwrap_or_else(|_| other(ty, text)),

        Type::DATE => parse_date_days(text)
            .map(Value::Date)
            .unwrap_or_else(|| other(ty, text)),

        Type::TIME => parse_time_micros(text)
            .map(Value::Time)
            .unwrap_or_else(|| other(ty, text)),

        Type::TIMESTAMP => parse_naive_micros(text)
            .map(|micros| Value::Timestamp { micros, tz: false })
            .unwrap_or_else(|| other(ty, text)),

        Type::TIMESTAMPTZ => parse_tz_micros(text)
            .map(|micros| Value::Timestamp { micros, tz: true })
            .unwrap_or_else(|| other(ty, text)),

        Type::INTERVAL => Value::Interval(text.to_string()),

        // Arrays, ranges, composites, enums, domains, extension types, `timetz`.
        _ => other(ty, text),
    }
}

/// Decode Postgres's default `bytea_output = hex` form: `\x48656c6c6f`.
/// Returns `None` for the `escape` form or malformed input, so the caller can
/// fall back to `Other` rather than silently producing wrong bytes.
fn decode_bytea_hex(text: &str) -> Option<Vec<u8>> {
    let hex = text.strip_prefix(r"\x")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        let s = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(s, 16).ok()?);
    }
    Some(out)
}

/// Days since the **Unix** epoch. Postgres's binary date epoch is 2000-01-01;
/// on the text path we parse a calendar date, so no epoch offset applies here.
fn parse_date_days(text: &str) -> Option<i32> {
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)?;
    Some((date - epoch).num_days() as i32)
}

/// Micros since midnight. Handles 0–6 fractional digits via `%.f`.
fn parse_time_micros(text: &str) -> Option<i64> {
    let t = NaiveTime::parse_from_str(text, "%H:%M:%S%.f").ok()?;
    Some(time_to_micros(t))
}

fn time_to_micros(t: NaiveTime) -> i64 {
    use chrono::Timelike;
    let secs = t.num_seconds_from_midnight() as i64;
    // `nanosecond()` can exceed 1e9 on a leap second; Postgres never emits one,
    // and clamping keeps this total rather than panicking if it ever does.
    let nanos = t.nanosecond().min(999_999_999) as i64;
    secs * MICROS_PER_SEC + nanos / 1_000
}

/// Wall-clock micros, interpreted as if UTC. NOT an instant.
fn parse_naive_micros(text: &str) -> Option<i64> {
    let dt = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").ok()?;
    naive_to_micros(dt)
}

fn naive_to_micros(dt: NaiveDateTime) -> Option<i64> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)?.and_hms_opt(0, 0, 0)?;
    let delta = dt.signed_duration_since(epoch);
    delta.num_microseconds()
}

/// A true instant, normalized to UTC by applying the offset present in the text.
/// `%#z` accepts both `+05` and `+05:30`, which is what Postgres emits.
fn parse_tz_micros(text: &str) -> Option<i64> {
    let dt = chrono::DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f%#z").ok()?;
    let secs = dt.timestamp();
    let subsec = dt.timestamp_subsec_micros() as i64;
    secs.checked_mul(MICROS_PER_SEC)?.checked_add(subsec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucent_protocol::Value;
    use tokio_postgres::types::Type;

    fn d(text: &str, ty: Type) -> Value {
        pg_text_to_value(text, &ty)
    }

    #[test]
    fn decodes_integers() {
        assert!(matches!(d("42", Type::INT2), Value::Int64(42)));
        assert!(matches!(d("42", Type::INT4), Value::Int64(42)));
        assert!(matches!(
            d("4200000000000", Type::INT8),
            Value::Int64(4200000000000)
        ));
        assert!(matches!(d("-1", Type::INT4), Value::Int64(-1)));
    }

    #[test]
    fn decodes_floats_including_nan_and_infinity() {
        match d("1.5", Type::FLOAT8) {
            Value::Float64(f) => assert_eq!(f, 1.5),
            o => panic!("{o:?}"),
        }
        match d("NaN", Type::FLOAT8) {
            Value::Float64(f) => assert!(f.is_nan()),
            o => panic!("{o:?}"),
        }
        match d("Infinity", Type::FLOAT8) {
            Value::Float64(f) => assert!(f.is_infinite() && f.is_sign_positive()),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn numeric_keeps_server_text_verbatim_including_nan_and_exponents() {
        assert!(matches!(d("1234.56", Type::NUMERIC), Value::Decimal(s) if s == "1234.56"));
        assert!(matches!(d("NaN", Type::NUMERIC), Value::Decimal(s) if s == "NaN"));
        assert!(matches!(d("1e10", Type::NUMERIC), Value::Decimal(s) if s == "1e10"));
        // A 38-digit value must survive exactly — this is the whole point.
        let big = "12345678901234567890.123456789012345678";
        assert!(matches!(d(big, Type::NUMERIC), Value::Decimal(s) if s == big));
    }

    #[test]
    fn decodes_bool_from_postgres_t_and_f() {
        assert!(matches!(d("t", Type::BOOL), Value::Bool(true)));
        assert!(matches!(d("f", Type::BOOL), Value::Bool(false)));
        // Anything else fails soft.
        assert!(matches!(d("maybe", Type::BOOL), Value::Other { .. }));
    }

    #[test]
    fn decodes_text_types() {
        assert!(matches!(d("hello", Type::TEXT), Value::Text(s) if s == "hello"));
        assert!(matches!(d("varchar", Type::VARCHAR), Value::Text(s) if s == "varchar"));
        assert!(matches!(d("ch", Type::BPCHAR), Value::Text(s) if s == "ch"));
    }

    #[test]
    fn decodes_bytea_hex_format() {
        // Postgres default bytea_output = hex
        assert!(
            matches!(d(r"\x48690a", Type::BYTEA), Value::Binary(b) if b == vec![0x48, 0x69, 0x0a])
        );
        assert!(matches!(d(r"\x", Type::BYTEA), Value::Binary(b) if b.is_empty()));
        // Escape format (bytea_output = escape) is not hex — fail soft.
        assert!(matches!(d("Hi", Type::BYTEA), Value::Other { .. }));
        // Odd digit count is malformed — fail soft.
        assert!(matches!(d(r"\x4", Type::BYTEA), Value::Other { .. }));
    }

    #[test]
    fn decodes_json_and_jsonb_as_json() {
        assert!(matches!(d(r#"{"a": 1}"#, Type::JSON), Value::Json(s) if s == r#"{"a": 1}"#));
        assert!(matches!(d(r#"{"a":1}"#, Type::JSONB), Value::Json(s) if s == r#"{"a":1}"#));
    }

    #[test]
    fn decodes_uuid() {
        let u = "00000000-0000-0000-0000-000000000000";
        assert!(matches!(d(u, Type::UUID), Value::Uuid(x) if x.is_nil()));
        assert!(matches!(d("not-a-uuid", Type::UUID), Value::Other { .. }));
    }

    #[test]
    fn date_is_days_since_the_unix_epoch() {
        assert!(matches!(d("1970-01-01", Type::DATE), Value::Date(0)));
        assert!(matches!(d("1970-01-02", Type::DATE), Value::Date(1)));
        assert!(matches!(d("1969-12-31", Type::DATE), Value::Date(-1)));
        // 2000-01-01 is the Postgres BINARY epoch. On the text path it is an
        // ordinary date; if this ever returns 0 we have leaked the PG epoch.
        assert!(matches!(d("2000-01-01", Type::DATE), Value::Date(10957)));
        assert!(matches!(d("garbage", Type::DATE), Value::Other { .. }));
    }

    #[test]
    fn time_is_micros_since_midnight_with_variable_fractional_digits() {
        assert!(matches!(d("00:00:00", Type::TIME), Value::Time(0)));
        assert!(matches!(
            d("12:34:56", Type::TIME),
            Value::Time(45_296_000_000)
        ));
        assert!(matches!(
            d("12:34:56.789", Type::TIME),
            Value::Time(45_296_789_000)
        ));
        assert!(matches!(
            d("12:34:56.789012", Type::TIME),
            Value::Time(45_296_789_012)
        ));
    }

    #[test]
    fn naive_timestamp_is_wall_clock_interpreted_as_utc() {
        match d("1970-01-01 00:00:01", Type::TIMESTAMP) {
            Value::Timestamp { micros, tz } => {
                assert_eq!(micros, 1_000_000);
                assert!(!tz, "timestamp without time zone must set tz = false");
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn timestamptz_parses_the_offset_present_in_the_text() {
        // 1970-01-01 05:30:00+05:30 is exactly the Unix epoch.
        match d("1970-01-01 05:30:00+05:30", Type::TIMESTAMPTZ) {
            Value::Timestamp { micros, tz } => {
                assert_eq!(micros, 0, "offset must be applied, not ignored");
                assert!(tz, "timestamptz must set tz = true");
            }
            o => panic!("{o:?}"),
        }
        // Postgres may emit a bare hour offset.
        match d("1970-01-01 00:00:00+00", Type::TIMESTAMPTZ) {
            Value::Timestamp { micros, .. } => assert_eq!(micros, 0),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn interval_and_unmapped_types_keep_their_text() {
        assert!(matches!(d("1 day", Type::INTERVAL), Value::Interval(s) if s == "1 day"));
        match d("[1,5)", Type::INT4_RANGE) {
            Value::Other { type_name, text } => {
                assert_eq!(type_name, "int4range");
                assert_eq!(text, "[1,5)");
            }
            o => panic!("expected Other, got {o:?}"),
        }
    }

    #[test]
    fn unparseable_known_oid_degrades_to_other_and_never_panics() {
        assert!(matches!(d("not-a-number", Type::INT4), Value::Other { .. }));
        assert!(matches!(
            d("not-a-float", Type::FLOAT8),
            Value::Other { .. }
        ));
        assert!(matches!(d("99:99:99", Type::TIME), Value::Other { .. }));
    }
}
