//! Real-Postgres integration tests for query_paging (feature-gated: `integration-tests`).
//! Usage: cargo test --package lucent --features integration-tests -- --nocapture

#![cfg(feature = "integration-tests")]

use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use tokio_postgres::NoTls;

use crate::query_paging::{wrap_for_count, wrap_for_page, FilterSpec, SortSpec};
use crate::sql_builder::PostgresSqlBuilder;

fn pg() -> PostgresSqlBuilder {
    PostgresSqlBuilder
}

async fn setup() -> (impl Drop, tokio_postgres::Client) {
    let c = testcontainers_modules::postgres::Postgres::default()
        .start()
        .await
        .expect("Postgres container");
    let port = c.get_host_port_ipv4(5432).await.unwrap();
    let conn_str =
        format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");

    let mut last_err = None;
    for i in 0..10 {
        match tokio_postgres::connect(&conn_str, NoTls).await {
            Ok((client, conn)) => {
                tokio::spawn(async move {
                    conn.await.ok();
                });
                client
                    .batch_execute(
                        "CREATE TABLE widgets (id SERIAL PRIMARY KEY, name TEXT, active BOOLEAN);
                         INSERT INTO widgets (name, active)
                         SELECT 'widget_' || g, g % 2 = 0 FROM generate_series(1, 500) g;",
                    )
                    .await
                    .expect("seed data");
                return (c, client);
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (i + 1))).await;
            }
        }
    }
    panic!("Postgres never became ready: {last_err:?}");
}

#[tokio::test]
async fn paginated_wrap_returns_exactly_the_requested_chunk() {
    let (_container, client) = setup().await;
    let sql = wrap_for_page("SELECT * FROM widgets", &None, &[], 50, 0, &pg());
    let rows = client.query(&sql, &[]).await.expect("query executes");
    assert_eq!(rows.len(), 50);
}

#[tokio::test]
async fn second_chunk_continues_where_the_first_left_off() {
    let (_container, client) = setup().await;
    let first = client
        .query(
            &wrap_for_page(
                "SELECT * FROM widgets ORDER BY id",
                &None,
                &[],
                50,
                0,
                &pg(),
            ),
            &[],
        )
        .await
        .unwrap();
    let second = client
        .query(
            &wrap_for_page(
                "SELECT * FROM widgets ORDER BY id",
                &None,
                &[],
                50,
                50,
                &pg(),
            ),
            &[],
        )
        .await
        .unwrap();
    let first_ids: Vec<i32> = first.iter().map(|r| r.get::<_, i32>(0)).collect();
    let second_ids: Vec<i32> = second.iter().map(|r| r.get::<_, i32>(0)).collect();
    assert!(first_ids.iter().max() < second_ids.iter().min());
}

#[tokio::test]
async fn sort_pushdown_orders_by_the_requested_column() {
    let (_container, client) = setup().await;
    let sort = Some(SortSpec {
        column: "id".into(),
        direction: "desc".into(),
    });
    let sql = wrap_for_page("SELECT * FROM widgets", &sort, &[], 5, 0, &pg());
    let rows = client.query(&sql, &[]).await.unwrap();
    let ids: Vec<i32> = rows.iter().map(|r| r.get::<_, i32>(0)).collect();
    assert_eq!(ids, vec![500, 499, 498, 497, 496]);
}

#[tokio::test]
async fn filter_pushdown_restricts_to_matching_rows() {
    let (_container, client) = setup().await;
    let filters = vec![FilterSpec {
        column: "active".into(),
        operator: "eq".into(),
        value: Some("false".into()),
    }];
    let sql = wrap_for_page("SELECT * FROM widgets", &None, &filters, 500, 0, &pg());
    let rows = client.query(&sql, &[]).await.unwrap();
    assert_eq!(rows.len(), 250); // odd-numbered widgets are active=false
}

#[tokio::test]
async fn count_all_matches_the_actual_row_total() {
    let (_container, client) = setup().await;
    let sql = wrap_for_count("SELECT * FROM widgets", &[], &pg());
    let rows = client.query(&sql, &[]).await.unwrap();
    let count: i64 = rows[0].get(0);
    assert_eq!(count, 500);
}

#[tokio::test]
async fn count_all_with_filter_matches_the_filtered_subset() {
    let (_container, client) = setup().await;
    let filters = vec![FilterSpec {
        column: "active".into(),
        operator: "eq".into(),
        value: Some("true".into()),
    }];
    let sql = wrap_for_count("SELECT * FROM widgets", &filters, &pg());
    let rows = client.query(&sql, &[]).await.unwrap();
    let count: i64 = rows[0].get(0);
    assert_eq!(count, 250);
}

#[tokio::test]
async fn contains_filter_with_percent_in_value_matches_literal_percent_not_wildcard() {
    let (_container, client) = setup().await;
    client
        .batch_execute("INSERT INTO widgets (name, active) VALUES ('50% off', true)")
        .await
        .unwrap();
    let filters = vec![FilterSpec {
        column: "name".into(),
        operator: "contains".into(),
        value: Some("50%".into()),
    }];
    let sql = wrap_for_page("SELECT * FROM widgets", &None, &filters, 10, 0, &pg());
    let rows = client.query(&sql, &[]).await.unwrap();
    assert_eq!(rows.len(), 1);
}
