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
        ConnectionConfig {
            host: "127.0.0.1".to_string(),
            port,
            user: "postgres".to_string(),
            password: "postgres".to_string(),
            database: "postgres".to_string(),
            ssl_mode: "prefer".to_string(),
        },
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
                '10.0.0.1/24'::cidr             AS cidr_val
            "#
            .to_string(),
            tx,
        )
        .await;

    let rows = single_row(&mut rx).await;

    assert_eq!(rows.len(), 22, "expected 22 result columns");

    // int2
    assert!(
        matches!(rows[0], Value::Int(42)),
        "int2: expected Int(42), got {:?}",
        rows[0]
    );
    // int4
    assert!(
        matches!(rows[1], Value::Int(42)),
        "int4: expected Int(42), got {:?}",
        rows[1]
    );
    // int8
    assert!(
        matches!(rows[2], Value::Int(v) if v == 4200000000000i64),
        "int8: expected Int(4200000000000), got {:?}",
        rows[2]
    );
    // float4
    assert!(
        matches!(rows[3], Value::Float(v) if (v - std::f64::consts::PI).abs() < 0.01),
        "float4: got {:?}",
        rows[3]
    );
    // float8
    assert!(
        matches!(rows[4], Value::Float(v) if (v - std::f64::consts::PI).abs() < 0.0001),
        "float8: got {:?}",
        rows[4]
    );
    // numeric
    assert!(
        matches!(rows[5], Value::Text(ref s) if s == "1234.56"),
        "numeric: expected Text(\"1234.56\"), got {:?}",
        rows[5]
    );
    // bool
    assert!(
        matches!(rows[6], Value::Bool(true)),
        "bool: got {:?}",
        rows[6]
    );
    // text
    assert!(
        matches!(rows[7], Value::Text(ref s) if s == "hello"),
        "text: got {:?}",
        rows[7]
    );
    // varchar
    assert!(
        matches!(rows[8], Value::Text(ref s) if s == "varchar"),
        "varchar: got {:?}",
        rows[8]
    );
    // jsonb
    assert!(
        matches!(rows[9], Value::Text(ref s) if s.contains("\"a\":1")),
        "jsonb: got {:?}",
        rows[9]
    );
    // json
    assert!(
        matches!(rows[10], Value::Text(ref s) if s.contains("\"a\":1")),
        "json: got {:?}",
        rows[10]
    );
    // uuid
    assert!(
        matches!(rows[11], Value::Text(ref s) if s.len() == 36),
        "uuid: got {:?}",
        rows[11]
    );
    // inet
    assert!(
        matches!(rows[12], Value::Text(ref s) if s == "192.168.1.1"),
        "inet: got {:?}",
        rows[12]
    );
    // macaddr
    assert!(
        matches!(rows[13], Value::Text(_)),
        "macaddr: got {:?}",
        rows[13]
    );
    // bytea
    assert!(
        matches!(rows[14], Value::Text(ref s) if s.contains("deadbeef")),
        "bytea: got {:?}",
        rows[14]
    );
    // date
    assert!(
        matches!(rows[15], Value::Text(_)),
        "date: got {:?}",
        rows[15]
    );
    // time
    assert!(
        matches!(rows[16], Value::Text(_)),
        "time: got {:?}",
        rows[16]
    );
    // timetz
    assert!(
        matches!(rows[17], Value::Text(ref s) if s.contains("+05:30") || s.contains("+0530")),
        "timetz: got {:?}",
        rows[17]
    );
    // timestamp
    assert!(
        matches!(rows[18], Value::Text(_)),
        "timestamp: got {:?}",
        rows[18]
    );
    // timestamptz
    assert!(
        matches!(rows[19], Value::Text(_)),
        "timestamptz: got {:?}",
        rows[19]
    );
    // interval
    assert!(
        matches!(rows[20], Value::Text(_)),
        "interval: got {:?}",
        rows[20]
    );
    // cidr
    assert!(
        matches!(rows[21], Value::Text(_)),
        "cidr: got {:?}",
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
        ConnectionConfig {
            host: "127.0.0.1".to_string(),
            port,
            user: "postgres".to_string(),
            password: "postgres".to_string(),
            database: "postgres".to_string(),
            ssl_mode: "prefer".to_string(),
        },
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
        matches!(total, Value::Text(ref s) if !s.is_empty()),
        "numeric sum should be a non-empty text: got {:?}",
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
        ConnectionConfig {
            host: "127.0.0.1".to_string(),
            port,
            user: "postgres".to_string(),
            password: "postgres".to_string(),
            database: "postgres".to_string(),
            ssl_mode: "prefer".to_string(),
        },
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
