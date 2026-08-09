#[cfg(test)]
mod integration {
    use lucent_lib::ai::context::{build_system_prompt, SchemaCache, SchemaNode, SchemaTree};
    use lucent_lib::ai::guard;
    use lucent_protocol::SqlDialect;

    fn schema(n_tables: usize) -> SchemaTree {
        SchemaTree {
            database_name: "db".into(),
            server_version: "PG16".into(),
            schemas: vec![SchemaNode {
                name: "public".into(),
                tables: (0..n_tables).map(|i| format!("t{i}")).collect(),
                views: vec![],
                functions: vec![],
            }],
        }
    }

    #[test]
    fn guard_full_pipeline() {
        assert!(guard::validate_readonly("SELECT * FROM users", SqlDialect::PostgreSql).is_ok());
        assert!(guard::validate_readonly(
            "WITH x AS (SELECT * FROM o) SELECT * FROM x",
            SqlDialect::PostgreSql
        )
        .is_ok());
        assert!(
            guard::validate_readonly("DELETE FROM users WHERE id=1", SqlDialect::PostgreSql)
                .is_err()
        );
        assert!(guard::validate_readonly(
            "EXPLAIN ANALYZE SELECT * FROM users",
            SqlDialect::PostgreSql
        )
        .is_err());
        assert!(guard::validate_readonly("SELECT 1; SELECT 2", SqlDialect::PostgreSql).is_err());
        assert!(
            guard::validate_readonly("INSERT INTO t VALUES (1)", SqlDialect::PostgreSql).is_err()
        );
    }

    #[test]
    fn blast_radius_extraction() {
        let sql = "DELETE FROM orders WHERE status='cancelled' AND created_at<'2024-01-01'";
        let c = guard::extract_where_for_count(sql, SqlDialect::PostgreSql).unwrap();
        assert!(c.contains("status") && c.contains("created_at"));
    }

    #[test]
    fn blast_radius_no_where_returns_none() {
        assert!(
            guard::extract_where_for_count("DELETE FROM orders", SqlDialect::PostgreSql).is_none()
        );
    }

    #[test]
    fn extract_table_name() {
        assert_eq!(
            guard::extract_table_name("DELETE FROM orders WHERE id=1", SqlDialect::PostgreSql)
                .as_deref(),
            Some("orders")
        );
        assert_eq!(
            guard::extract_table_name("UPDATE users SET a=1 WHERE id=1", SqlDialect::PostgreSql)
                .as_deref(),
            Some("users")
        );
    }

    #[test]
    fn small_schema_verbose() {
        let p = build_system_prompt(&schema(2), None, false, None);
        assert!(p.contains("t0") && p.contains("t1"));
    }

    #[test]
    fn large_schema_compact_shows_count_not_names() {
        let schemas: Vec<SchemaNode> = (0..80)
            .map(|i| SchemaNode {
                name: format!("s{i}"),
                tables: (0..10).map(|j| format!("t{i}_{j}")).collect(),
                views: vec![],
                functions: vec![],
            })
            .collect();
        let schema = SchemaTree {
            database_name: "big".into(),
            server_version: "PG16".into(),
            schemas,
        };
        let p = build_system_prompt(&schema, None, false, None);
        assert!(p.contains("10 tables"));
        assert!(!p.contains("t0_0"), "individual names must not appear");
    }

    #[test]
    fn cache_ttl_invalidation() {
        let c = SchemaCache::new(9999);
        c.set("x".into(), schema(1));
        assert!(c.get("x").is_some());
        c.invalidate("x");
        assert!(c.get("x").is_none());
    }
}
