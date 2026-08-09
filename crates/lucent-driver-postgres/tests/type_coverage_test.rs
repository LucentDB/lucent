use std::time::Duration;

use lucent_driver_postgres::PostgresConnector;
use lucent_protocol::{ConnectionConfig, ConnectionId, QueryId, ResultShape, Value};
use lucent_worker_host::ExecutionEvent;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::mpsc;
use uuid::Uuid;

async fn connect_with_retry(
    connector: &PostgresConnector,
    connection_id: ConnectionId,
    config: ConnectionConfig,
) {
    for i in 0..10 {
        match connector.connect(connection_id, config.clone()).await {
            Ok(_) => return,
            Err(_) if i < 9 => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => panic!("connect failed after 10 retries: {e}"),
        }
    }
}

async fn single_row(rx: &mut mpsc::Receiver<ExecutionEvent>) -> Vec<Value> {
    let event = rx.recv().await.unwrap();
    match event {
        ExecutionEvent::Batch(ResultShape::Tabular { rows, .. }, _) => {
            assert_eq!(rows.len(), 1, "expected exactly 1 row");
            rows.into_iter().next().unwrap()
        }
        ExecutionEvent::Failed(e) => panic!("query failed: {e}"),
        _ => panic!("unexpected non-Tabular event"),
    }
}

#[tokio::test]
async fn all_common_types_round_trip_correctly() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());
    connect_with_retry(
        &connector,
        connection_id,
        ConnectionConfig::new("postgres")
            .with("host", "127.0.0.1")
            .with("port", port.to_string())
            .with("user", "postgres")
            .with("database", "postgres")
            .with("ssl_mode", "prefer")
            .with_secret("postgres"),
    )
    .await;

    let (tx, mut rx) = mpsc::channel(4);

    connector
        .execute(
            connection_id,
            QueryId(Uuid::new_v4()),
            r#"
            SELECT
                42::int2                        AS int2_val,
                42::int4                        AS int4_val,
                4200000000000::int8              AS int8_val,
                3.14::float4                    AS float4_val,
                3.14159265358979::float8        AS float8_val,
                1234.56::numeric(10,2)          AS numeric_val,
                true::bool                      AS bool_val,
                'hello'::text                   AS text_val,
                'varchar'::varchar(20)          AS varchar_val,
                '{"a":1}'::jsonb                AS jsonb_val,
                '{"a":1}'::json                 AS json_val,
                'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid AS uuid_val,
                '192.168.1.1'::inet             AS inet_val,
                '00:11:22:33:44:55'::macaddr    AS macaddr_val,
                E'\\xdeadbeef'::bytea           AS bytea_val,
                '2024-01-15'::date              AS date_val,
                '14:30:00'::time                AS time_val,
                '14:30:00+05:30'::timetz        AS timetz_val,
                '2024-01-15 10:30:00'::timestamp               AS ts_val,
                '2024-01-15 10:30:00+00'::timestamptz          AS tstz_val,
                '1 year 2 months'::interval     AS interval_val,
                '10.0.0.0/24'::cidr             AS cidr_val
            "#
            .to_string(),
            tx,
        )
        .await;

    let rows = single_row(&mut rx).await;

    assert_eq!(rows.len(), 22, "expected 22 result columns");

    // 0-2: integers
    assert!(matches!(rows[0], Value::Int64(42)), "int2: {:?}", rows[0]);
    assert!(matches!(rows[1], Value::Int64(42)), "int4: {:?}", rows[1]);
    assert!(
        matches!(rows[2], Value::Int64(4200000000000)),
        "int8: {:?}",
        rows[2]
    );

    // 3-4: floats
    assert!(
        matches!(rows[3], Value::Float64(f) if (f - 3.14).abs() < 1e-5),
        "float4: {:?}",
        rows[3]
    );
    assert!(
        matches!(rows[4], Value::Float64(f) if (f - 3.14159265358979).abs() < 1e-12),
        "float8: {:?}",
        rows[4]
    );

    // 5: numeric keeps the server's text verbatim
    assert!(
        matches!(rows[5], Value::Decimal(ref s) if s == "1234.56"),
        "numeric: {:?}",
        rows[5]
    );

    // 6: bool — the wire text is "t", the value is typed
    assert!(matches!(rows[6], Value::Bool(true)), "bool: {:?}", rows[6]);

    // 7-8: text types stay Text
    assert!(
        matches!(rows[7], Value::Text(ref s) if s == "hello"),
        "text: {:?}",
        rows[7]
    );
    assert!(
        matches!(rows[8], Value::Text(ref s) if s == "varchar"),
        "varchar: {:?}",
        rows[8]
    );

    // 9: jsonb — Postgres normalizes and re-renders with a space
    assert!(
        matches!(rows[9], Value::Json(ref s) if s.contains("\"a\": 1")),
        "jsonb: {:?}",
        rows[9]
    );
    // 10: json — stored and returned verbatim
    assert!(
        matches!(rows[10], Value::Json(ref s) if s.contains("\"a\":1")),
        "json: {:?}",
        rows[10]
    );

    // 11: uuid
    assert!(
        matches!(rows[11], Value::Uuid(u) if u.to_string() == "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"),
        "uuid: {:?}",
        rows[11]
    );

    // 12-13: inet and macaddr are not mapped — they take the escape hatch
    assert!(
        matches!(rows[12], Value::Other { ref type_name, ref text } if type_name == "inet" && text == "192.168.1.1"),
        "inet: {:?}",
        rows[12]
    );
    assert!(
        matches!(rows[13], Value::Other { ref type_name, .. } if type_name == "macaddr"),
        "macaddr: {:?}",
        rows[13]
    );

    // 14: bytea, hex output form
    assert!(
        matches!(rows[14], Value::Binary(ref b) if b == &vec![0xde, 0xad, 0xbe, 0xef]),
        "bytea: {:?}",
        rows[14]
    );

    // 15: date — 2024-01-15 is 19737 days after the Unix epoch.
    // If this ever reads 8781 we have leaked Postgres's 2000-01-01 binary epoch.
    assert!(
        matches!(rows[15], Value::Date(19737)),
        "date: {:?}",
        rows[15]
    );

    // 16: time — 14:30:00 = 52,200 s past midnight
    assert!(
        matches!(rows[16], Value::Time(52_200_000_000)),
        "time: {:?}",
        rows[16]
    );

    // 17: timetz is deliberately NOT Value::Time — it carries a zone, Time does not
    assert!(
        matches!(rows[17], Value::Other { ref type_name, .. } if type_name == "timetz"),
        "timetz: {:?}",
        rows[17]
    );

    // 18: timestamp — wall clock, no zone
    assert!(
        matches!(rows[18], Value::Timestamp { tz: false, .. }),
        "timestamp: {:?}",
        rows[18]
    );
    // 19: timestamptz — a true instant; +00 offset means this equals the naive value
    assert!(
        matches!(rows[19], Value::Timestamp { tz: true, .. }),
        "timestamptz: {:?}",
        rows[19]
    );
    // Both describe the same moment here, so their micros must agree.
    match (&rows[18], &rows[19]) {
        (Value::Timestamp { micros: a, .. }, Value::Timestamp { micros: b, .. }) => {
            assert_eq!(
                a, b,
                "a +00 timestamptz must equal the same naive timestamp"
            );
        }
        other => panic!("expected two timestamps, got {other:?}"),
    }

    // 20: interval keeps its text
    assert!(
        matches!(rows[20], Value::Interval(ref s) if s.contains("1 year")),
        "interval: {:?}",
        rows[20]
    );

    // 21: cidr is not mapped
    assert!(
        matches!(rows[21], Value::Other { ref type_name, .. } if type_name == "cidr"),
        "cidr: {:?}",
        rows[21]
    );
}

#[tokio::test]
async fn numeric_values_are_not_null() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());
    connect_with_retry(
        &connector,
        connection_id,
        ConnectionConfig::new("postgres")
            .with("host", "127.0.0.1")
            .with("port", port.to_string())
            .with("user", "postgres")
            .with("database", "postgres")
            .with("ssl_mode", "prefer")
            .with_secret("postgres"),
    )
    .await;

    let (tx, mut rx) = mpsc::channel(4);
    connector
        .execute(
            connection_id,
            QueryId(Uuid::new_v4()),
            // Realistic query similar to what the LLM writes
            "SELECT SUM(1234.56::numeric(10,2)) AS total".to_string(),
            tx,
        )
        .await;

    let rows = single_row(&mut rx).await;
    assert_eq!(rows.len(), 1);
    let total = &rows[0];
    assert!(
        !matches!(total, Value::Null),
        "SUM of numeric must not be Null — got Null"
    );
    assert!(
        matches!(total, Value::Decimal(ref s) if !s.is_empty()),
        "numeric sum should be a non-empty decimal: got {:?}",
        total
    );
}

#[tokio::test]
async fn null_values_are_null_not_sentinel() {
    // Ensure the fallback sentinel `<type_name>` is never used for NULL database values
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());
    connect_with_retry(
        &connector,
        connection_id,
        ConnectionConfig::new("postgres")
            .with("host", "127.0.0.1")
            .with("port", port.to_string())
            .with("user", "postgres")
            .with("database", "postgres")
            .with("ssl_mode", "prefer")
            .with_secret("postgres"),
    )
    .await;

    let (tx, mut rx) = mpsc::channel(4);
    connector
        .execute(
            connection_id,
            QueryId(Uuid::new_v4()),
            "SELECT NULL::int4, NULL::text, NULL::numeric, NULL::bool".to_string(),
            tx,
        )
        .await;

    let rows = single_row(&mut rx).await;
    assert_eq!(rows.len(), 4);
    for (i, val) in rows.iter().enumerate() {
        assert!(
            matches!(val, Value::Null),
            "column {i} should be Null, got {val:?}"
        );
    }
}

#[tokio::test]
async fn numeric_precision_and_int8_range_survive_the_wire() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());
    connect_with_retry(
        &connector,
        connection_id,
        ConnectionConfig::new("postgres")
            .with("host", "127.0.0.1")
            .with("port", port.to_string())
            .with("user", "postgres")
            .with("database", "postgres")
            .with("ssl_mode", "disable")
            .with_secret("postgres"),
    )
    .await;

    let (tx, mut rx) = mpsc::channel(4);
    connector
        .execute(
            connection_id,
            QueryId(Uuid::new_v4()),
            "SELECT 12345678901234567890.123456789012345678::numeric AS n, \
                    9223372036854775807::int8 AS i"
                .into(),
            tx,
        )
        .await;

    let row = single_row(&mut rx).await;

    match &row[0] {
        Value::Decimal(s) => assert_eq!(
            s, "12345678901234567890.123456789012345678",
            "numeric must survive verbatim — never through a float"
        ),
        o => panic!("expected Decimal, got {o:?}"),
    }
    match &row[1] {
        Value::Int64(i) => assert_eq!(*i, i64::MAX, "int8 must survive its full range"),
        o => panic!("expected Int64, got {o:?}"),
    }
}
