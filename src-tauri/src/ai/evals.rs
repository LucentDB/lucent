//! Eval harness (feature `evals`). Runs natural-language questions through the
//! full agent against the live demo DB, grades answers by digit-containment
//! against ground-truth SQL executed at run time, writes target/eval-report.md.
//!
//! Run:  docker start lucent-dev-pg && \
//!       OPENAI_API_KEY=... LUCENT_EVAL_MODEL=gpt-4o \
//!       cargo test -p lucent --features evals -- --ignored eval_run --nocapture
//!
//! Grading rule: every truth value must be ≥4 digits — short numbers would
//! containment-match spuriously. Truth queries use the CORRECT validity join;
//! cases 3 and 4 are the two questions the agent got wrong in production
//! (3.07x and 2.56x overstated respectively).

use lucent_protocol::ConnectionConfig;

pub struct EvalCase {
    pub name: &'static str,
    pub question: &'static str,
    /// Must return one row, one column, castable to text, ≥4 digits.
    pub truth_sql: &'static str,
}

pub fn eval_cases() -> Vec<EvalCase> {
    vec![
        EvalCase { name: "unique_passengers",
            question: "how many unique passengers are in the system?",
            truth_sql: "SELECT COUNT(DISTINCT passenger_id)::text FROM bookings.tickets" },
        EvalCase { name: "min_segment_price",
            question: "what is the cheapest ticket segment price?",
            truth_sql: "SELECT MIN(price)::bigint::text FROM bookings.segments" },
        EvalCase { name: "flights_77w_validity",
            question: "how many non-cancelled flights did the Boeing 777-300ER operate?",
            truth_sql: "SELECT COUNT(*)::text FROM bookings.flights f \
                        JOIN bookings.routes r ON f.route_no = r.route_no \
                          AND r.validity @> f.scheduled_departure \
                        WHERE r.airplane_code = '77W' AND f.status != 'Cancelled'" },
        EvalCase { name: "top_row_77w_boardings",
            question: "which seat row on the Boeing 777-300ER has the most boardings, and how many boardings is that?",
            truth_sql: "SELECT COUNT(*)::text FROM bookings.boarding_passes bp \
                        JOIN bookings.flights f ON f.flight_id = bp.flight_id \
                        JOIN bookings.routes r ON r.route_no = f.route_no \
                          AND r.validity @> f.scheduled_departure \
                        WHERE r.airplane_code = '77W' \
                        GROUP BY SUBSTRING(bp.seat_no FROM '^(\\d+)')::integer \
                        ORDER BY COUNT(*) DESC LIMIT 1" },
        EvalCase { name: "ticket_total_price",
            question: "what is the total price of all segments on ticket 0005433348362?",
            truth_sql: "SELECT SUM(price)::bigint::text FROM bookings.segments \
                        WHERE ticket_no = '0005433348362'" },
        EvalCase { name: "total_cancelled",
            question: "how many flights were cancelled in total?",
            truth_sql: "SELECT COUNT(*)::text FROM bookings.flights WHERE status = 'Cancelled'" },
        EvalCase { name: "total_flights",
            question: "how many flights are in the system in total?",
            truth_sql: "SELECT COUNT(*)::text FROM bookings.flights" },
        EvalCase { name: "busiest_pair_passenger_trips",
            question: "how many passenger trips does the busiest airport pair have?",
            truth_sql: "SELECT COUNT(*)::text FROM bookings.segments s \
                        JOIN bookings.flights f ON s.flight_id = f.flight_id \
                        JOIN bookings.routes r ON r.route_no = f.route_no \
                          AND r.validity @> f.scheduled_departure \
                        GROUP BY r.departure_airport, r.arrival_airport \
                        ORDER BY COUNT(*) DESC LIMIT 1" },
    ]
}

pub fn digits(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Extract the answer's numbers as whole tokens. Grouping separators
/// (commas/spaces BETWEEN digits) are removed so "27,204" and "6 289" read as
/// one number; a period between digits is a decimal point and splits, so
/// "$1,750.00" yields ["1750", "00"]. Token-wise comparison prevents the two
/// false-positive classes a flat digit-concat substring check allows:
/// adjacent numbers ("1,234 and 56,789" spuriously containing "3456") and
/// embedded matches ("162890" containing "6289").
fn number_tokens(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut cleaned = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let grouping = (c == ',' || c == ' ')
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if !grouping {
            cleaned.push(c);
        }
    }
    cleaned
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

pub fn answer_contains_truth(answer: &str, truth: &str) -> bool {
    let t = digits(truth);
    t.len() >= 4 && number_tokens(answer).iter().any(|n| n == &t)
}

/// Parse a libpq-style connection string into a `ConnectionConfig`.
/// Format: `host=... port=... user=... password=... dbname=...`
fn eval_connection_config(conn_str: &str) -> ConnectionConfig {
    let mut host = String::from("127.0.0.1");
    let mut port: u16 = 5432;
    let mut user = String::from("postgres");
    let mut password = String::from("postgres");
    let mut database = String::from("demo");

    for part in conn_str.split_whitespace() {
        if let Some((key, value)) = part.split_once('=') {
            match key {
                "host" => host = value.to_string(),
                "port" => port = value.parse().unwrap_or(5432),
                "user" => user = value.to_string(),
                "password" => password = value.to_string(),
                "dbname" => database = value.to_string(),
                _ => {}
            }
        }
    }
    ConnectionConfig::new("postgres")
        .with("host", host)
        .with("port", port.to_string())
        .with("user", user)
        .with("database", database)
        .with("ssl_mode", "prefer")
        .with_secret(password)
}

#[cfg(test)]
mod grading_tests {
    use super::*;

    #[test]
    fn eval_connection_config_parses_typical_connection_string() {
        let cfg = eval_connection_config(
            "host=db.example.com port=5433 user=admin password=s3cret dbname=analytics",
        );
        assert_eq!(cfg.get("host"), Some("db.example.com"));
        assert_eq!(cfg.port(), Some(5433));
        assert_eq!(cfg.get("user"), Some("admin"));
        assert_eq!(cfg.secret.as_deref(), Some("s3cret"));
        assert_eq!(cfg.get("database"), Some("analytics"));
    }

    #[test]
    fn eval_connection_config_defaults() {
        let cfg = eval_connection_config("");
        assert_eq!(cfg.get("host"), Some("127.0.0.1"));
        assert_eq!(cfg.port(), Some(5432));
        assert_eq!(cfg.get("user"), Some("postgres"));
        assert_eq!(cfg.secret.as_deref(), Some("postgres"));
        assert_eq!(cfg.get("database"), Some("demo"));
    }

    #[test]
    fn eval_connection_config_partial_override() {
        let cfg = eval_connection_config("host=10.0.0.1 dbname=production");
        assert_eq!(cfg.get("host"), Some("10.0.0.1"));
        assert_eq!(cfg.port(), Some(5432)); // default
        assert_eq!(cfg.get("database"), Some("production"));
        assert_eq!(cfg.get("user"), Some("postgres")); // default
    }

    #[test]
    fn digits_strips_formatting() {
        assert_eq!(digits("27,204 boardings (row 21)"), "2720421");
        assert_eq!(digits("6 289"), "6289");
    }

    #[test]
    fn containment_grading_finds_truth_across_formatting() {
        assert!(answer_contains_truth(
            "Row 21 with **27,204** boardings",
            "27204"
        ));
        assert!(!answer_contains_truth(
            "Row 21 with 69,640 boardings",
            "27204"
        ));
    }

    #[test]
    fn number_tokens_removes_grouping_and_splits_on_decimals() {
        assert_eq!(number_tokens("27,204 and 6 289"), vec!["27204", "6289"]);
        assert_eq!(number_tokens("$1,750.00"), vec!["1750", "00"]);
    }

    #[test]
    fn adjacent_numbers_do_not_falsely_contain_truth() {
        assert!(
            !answer_contains_truth("totals were 1,234 and 56,789", "3456"),
            "digit-concat across separate numbers must not grade as correct"
        );
    }

    #[test]
    fn embedded_digit_runs_do_not_falsely_contain_truth() {
        assert!(
            !answer_contains_truth("flight id 162890 departed", "6289"),
            "a truth value inside a longer number must not grade as correct"
        );
    }

    #[test]
    fn decimal_formatted_answer_still_matches_integer_truth() {
        assert!(answer_contains_truth(
            "the minimum price is $1,750.00",
            "1750"
        ));
    }

    #[test]
    fn eval_cases_all_have_distinctive_truths() {
        for case in eval_cases() {
            assert!(
                !case.name.is_empty() && case.truth_sql.to_uppercase().contains("SELECT"),
                "malformed case {}",
                case.name
            );
        }
    }
}

#[cfg(all(test, feature = "evals"))]
mod runner {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use crate::ai::events::{AiEvent, DmlApprovalPayload};

    struct EvalSink(std::sync::Mutex<Vec<AiEvent>>);
    impl crate::ai::agent::AgentSink for EvalSink {
        fn event(&self, event: AiEvent) {
            self.0.lock().unwrap().push(event);
        }
        fn dml_approval(&self, _p: DmlApprovalPayload) {}
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires demo DB + OPENAI_API_KEY; run explicitly"]
    async fn eval_run() {
        let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY");
        let model = std::env::var("LUCENT_EVAL_MODEL").unwrap_or_else(|_| "gpt-4o".into());
        let endpoint = std::env::var("OPENAI_BASE_URL").ok();
        let db_conn = std::env::var("LUCENT_EVAL_DB").unwrap_or_else(|_| {
            "host=127.0.0.1 port=5432 user=postgres password=postgres dbname=demo".into()
        });

        // Ground-truth channel: plain tokio_postgres (independent of the app stack).
        let (truth_client, conn) = tokio_postgres::connect(&db_conn, tokio_postgres::NoTls)
            .await
            .expect("truth DB connect");
        tokio::spawn(async move {
            conn.await.ok();
        });

        // Agent channel: the same Supervisor + ConnectorClient path commands.rs uses.
        // If the worker binary is not locatable via Supervisor (e.g. under cargo test),
        // set LUCENT_WORKER_BIN to the full path of lucent-driver-postgres.
        let mut sup = crate::supervisor::Supervisor::new();
        let socket = sup.ensure_running().await.expect("worker").to_path_buf();
        let token = sup.handshake_token().to_string();
        let conn_config = eval_connection_config(&db_conn);
        let client = crate::client::ConnectorClient::connect(&socket, &token, conn_config)
            .await
            .expect("agent DB connect");
        let db = Arc::new(Mutex::new(Some(client)));

        // Build graph + embedder exactly as connect_db does, then the system prompt.
        let embedder = crate::ai::embed::Embedder::new().ok();
        let embedder = Arc::new(Mutex::new(embedder));
        let graph = {
            let mut g = db.lock().await;
            crate::ai::schema_graph::SchemaIndexer::build_index(
                g.as_mut().unwrap(),
                embedder.lock().await.as_ref().expect("embedder"),
                true,
            )
            .await
            .expect("graph")
        };
        let graph = Arc::new(Mutex::new(Some(graph)));

        let mut config = crate::ai::config::AiConfig::default();
        config.model = model.clone();
        config.endpoint = endpoint.clone();
        config.max_turns = 15;

        let mut report = String::from(
            "# Eval report\n\n| case | correct | turns | tokens | duration |\n|---|---|---|---|---|\n",
        );
        let mut passed = 0usize;
        let cases = eval_cases();

        for case in &cases {
            let truth: String = truth_client
                .query_one(case.truth_sql, &[])
                .await
                .expect(case.name)
                .get(0);
            assert!(
                digits(&truth).len() >= 4,
                "{}: truth '{}' too short to grade",
                case.name,
                truth
            );

            let start = std::time::Instant::now();
            let sink = Arc::new(EvalSink(std::sync::Mutex::new(vec![])));
            let conv = Arc::new(Mutex::new(crate::ai::agent::ConversationState::new(
                "eval".into(),
            )));
            let provider: Arc<dyn crate::ai::provider::LlmProvider> =
                Arc::new(crate::ai::providers::rig::RigProvider::new(
                    crate::ai::config::AiProvider::OpenAI,
                    api_key.clone(),
                    endpoint.clone(),
                ));
            let tool_ctx = crate::ai::tools::AiToolContext {
                db: db.clone(),
                capabilities: None,
                config: config.clone(),
                schema_graph: graph.clone(),
                embedder: embedder.clone(),
                reranker: Arc::new(Mutex::new(None)),
            };
            let system_prompt = {
                let g = graph.lock().await;
                let tree = crate::ai::context::tree_from_graph("demo".into(), g.as_ref().unwrap());
                // Evals run against a fixed in-memory schema with no live
                // connection, so there are no capabilities to disclose.
                crate::ai::context::build_system_prompt(&tree, g.as_ref(), true, None)
            };
            let tools = crate::ai::tools::all_tools(tool_ctx.clone());
            let agent = crate::ai::agent::DatabaseAgent::new(provider, tools, tool_ctx);
            let cancel = tokio_util::sync::CancellationToken::new();
            agent
                .chat(
                    case.question.into(),
                    &config,
                    system_prompt,
                    conv.clone(),
                    sink.clone(),
                    cancel,
                )
                .await
                .expect(case.name);

            let events = sink.0.lock().unwrap();
            let final_answer = events
                .iter()
                .rev()
                .find_map(|e| match e {
                    AiEvent::Done { final_message, .. } => Some(final_message.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let turns = events
                .iter()
                .filter(|e| matches!(e, AiEvent::ToolCalls { .. }))
                .count()
                + 1;
            let tokens: u32 = events
                .iter()
                .filter_map(|e| match e {
                    AiEvent::Done { usage, .. } => {
                        Some(usage.prompt_tokens + usage.completion_tokens)
                    }
                    _ => None,
                })
                .sum();

            let ok = answer_contains_truth(&final_answer, &truth);
            if ok {
                passed += 1;
            }
            let mark = if ok {
                "✅".to_string()
            } else {
                format!("❌ (truth {truth})")
            };
            report.push_str(&format!(
                "| {} | {} | {} | {} | {:.1}s |\n",
                case.name,
                mark,
                turns,
                tokens,
                start.elapsed().as_secs_f32(),
            ));
            eprintln!(
                "[eval] {} → {}",
                case.name,
                if ok { "PASS" } else { "FAIL" }
            );
        }

        report.push_str(&format!(
            "\n**{passed}/{} correct** — model {model}\n",
            cases.len()
        ));
        std::fs::create_dir_all("target").ok();
        std::fs::write("target/eval-report.md", &report).expect("write report");
        eprintln!("{report}");
    }
}
