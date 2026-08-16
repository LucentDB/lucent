//! Pre-flight grounding: local, deterministic context computed BEFORE the
//! first LLM call of a chat turn. Converts the model's schema-exploration
//! round trips (~3–4s of LLM latency each) into a <300ms local pre-step.

use lucent_protocol::ConnectionId;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ai::embed::Embedder;
use crate::ai::mschema::ContextTier;
use crate::ai::retrieval::Retriever;
use crate::ai::schema_graph::SchemaGraph;
use crate::client::ConnectorClient;

const MAX_LITERALS: usize = 8;
const MAX_VALUE_HINTS: usize = 5;
const PREFLIGHT_TOP_K: usize = 10;
const MAX_PROBE_COLUMNS_PER_LITERAL: usize = 4;
const PROBE_TIMEOUT_MS: u64 = 500;
const TEXTUAL_TYPES: &[&str] = &[
    "text",
    "character varying",
    "character",
    "varchar",
    "bpchar",
];

/// Words that start questions and would otherwise pass the capitalization test.
const STOPWORDS: &[&str] = &[
    "What", "Which", "Who", "Where", "When", "Why", "How", "Show", "List", "Find", "Get", "Give",
    "Count", "Top", "All", "The", "Is", "Are", "Do", "Does", "Can", "Please", "Select", "From",
    "And", "Or", "Not", "For",
];

/// Character-class shape of a value: digits→9, uppercase→A, lowercase→a.
/// "PG0072" → "AA9999". Used to pick which columns are worth probing for a
/// literal — a 13-digit literal only makes sense against columns whose
/// sampled values are also 13 digits.
pub fn shape_of(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                '9'
            } else if c.is_uppercase() {
                'A'
            } else if c.is_lowercase() {
                'a'
            } else {
                c
            }
        })
        .collect()
}

/// Columns worth probing for a literal: textual columns whose sample values
/// share the literal's shape, plus unsampled PK/FK textual columns (usually
/// indexed, so equality probes are cheap even on huge tables).
pub fn probe_candidates<'a>(
    graph: &'a crate::ai::schema_graph::SchemaGraph,
    literal: &str,
) -> Vec<&'a crate::ai::schema_graph::ColumnEntry> {
    let literal_shape = shape_of(literal);
    let mut out: Vec<&crate::ai::schema_graph::ColumnEntry> = Vec::new();
    for c in &graph.columns {
        if !TEXTUAL_TYPES.contains(&c.data_type.as_str()) {
            continue;
        }
        let shape_hit = c.sample_values.iter().any(|v| shape_of(v) == literal_shape);
        let unsampled_key = c.sample_values.is_empty() && (c.is_primary_key || c.fk_ref.is_some());
        if shape_hit || unsampled_key {
            out.push(c);
        }
        if out.len() >= MAX_PROBE_COLUMNS_PER_LITERAL {
            break;
        }
    }
    out
}

/// One single-statement batched probe: for each candidate column, an
/// index-friendly equality check against the literal's case variants.
pub fn build_probe_sql(
    candidates: &[&crate::ai::schema_graph::ColumnEntry],
    literal: &str,
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let mut variants: Vec<String> = vec![
        literal.to_string(),
        literal.to_uppercase(),
        literal.to_lowercase(),
    ];
    variants.dedup();
    let in_list = variants
        .iter()
        .map(|v| format!("'{}'", v.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");

    let subqueries: Vec<String> = candidates
        .iter()
        .map(|c| {
            let qualified = format!(
                "\"{}\".\"{}\"",
                c.schema.replace('"', "\"\""),
                c.table.replace('"', "\"\"")
            );
            let col = c.name.replace('"', "\"\"");
            let tag = format!("{}.{}.{}", c.schema, c.table, c.name).replace('\'', "''");
            format!(
                "(SELECT '{tag}' AS col, \"{col}\"::text AS val FROM {qualified} \
                  WHERE \"{col}\" IN ({in_list}) LIMIT 1)"
            )
        })
        .collect();
    Some(subqueries.join(" UNION ALL "))
}

/// Extract question tokens that plausibly correspond to stored values:
/// quoted spans, ALL-CAPS codes, digit-bearing tokens, Capitalized names.
pub fn extract_literals(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // Quoted spans first ('…' or "…"), kept whole.
    for quote in ['\'', '"'] {
        let mut rest = question; // scan the full question for each quote kind
        while let Some(start) = rest.find(quote) {
            let after = &rest[start + 1..];
            match after.find(quote) {
                Some(end) => {
                    let span = &after[..end];
                    if !span.is_empty() && !out.contains(&span.to_string()) {
                        out.push(span.to_string());
                    }
                    rest = &after[end + 1..];
                }
                None => break,
            }
        }
    }

    // Unquoted tokens.
    for raw in question.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if raw.len() < 2 || out.iter().any(|l| l == raw) {
            continue;
        }
        if STOPWORDS.contains(&raw) {
            continue;
        }
        let has_digit = raw.chars().any(|c| c.is_ascii_digit());
        let all_caps = raw
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && raw.chars().filter(|c| c.is_ascii_uppercase()).count() >= 2;
        let capitalized = raw.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && raw.chars().skip(1).any(|c| c.is_ascii_lowercase());
        if has_digit || all_caps || capitalized {
            out.push(raw.to_string());
        }
        if out.len() >= MAX_LITERALS {
            break;
        }
    }
    out
}

/// Match extracted literals against sampled column values (case-insensitive).
/// Hints show the STORED casing so the model writes the literal that actually
/// matches — casing mismatches are a top cause of silently-empty results.
const CONTAINMENT_MIN_CHARS: usize = 4;

/// Match extracted literals against sampled column values (case-insensitive).
/// First tries exact match; if the literal is ≥4 chars and no exact match,
/// falls back to substring containment — this grounds jsonb-extracted values
/// whose sampled form includes surrounding context (e.g. literal "777-300ER"
/// inside stored value "Boeing 777-300ER").
pub fn match_values(graph: &SchemaGraph, literals: &[String]) -> Vec<String> {
    let mut hints: Vec<String> = Vec::new();
    for lit in literals {
        let lower = lit.to_lowercase();
        for c in &graph.columns {
            let exact = c.sample_values.iter().find(|v| v.to_lowercase() == lower);
            let contained = if lit.len() >= CONTAINMENT_MIN_CHARS {
                c.sample_values
                    .iter()
                    .find(|v| v.to_lowercase().contains(&lower))
            } else {
                None
            };
            if let Some(stored) = exact.or(contained) {
                hints.push(format!(
                    "'{stored}' is a stored value of {}.{}.{}",
                    c.schema, c.table, c.name
                ));
                break; // one hint per literal
            }
        }
        if hints.len() >= MAX_VALUE_HINTS {
            break;
        }
    }
    hints
}

/// Combine the retrieved cluster section and value hints into one labelled
/// block, or None when there is nothing useful to say.
pub fn assemble_block(cluster_section: Option<String>, value_hints: &[String]) -> Option<String> {
    if cluster_section.is_none() && value_hints.is_empty() {
        return None;
    }
    let mut parts = vec![
        "[Schema context — retrieved automatically for this question; verify with tools if it seems incomplete]".to_string(),
    ];
    if let Some(section) = cluster_section {
        parts.push(section);
    }
    if !value_hints.is_empty() {
        parts.push("Value hints:".into());
        for h in value_hints {
            parts.push(format!("- {h}"));
        }
    }
    Some(parts.join("\n"))
}

/// Probe live data for literals that sample matching couldn't ground.
/// Best-effort: bounded by a 500ms statement timeout, fails silently.
///
/// `capabilities` decides whether probing is safe at all: with no known way
/// to open a bounded read-only scope the probes would run unprotected, so we
/// skip them entirely instead.
async fn probe_literals(
    connection_id: ConnectionId,
    db: &Arc<Mutex<Option<ConnectorClient>>>,
    graph: &crate::ai::schema_graph::SchemaGraph,
    literals: &[String],
    capabilities: Option<&lucent_protocol::DriverCapabilities>,
) -> Vec<String> {
    let mut hints: Vec<String> = Vec::new();
    let Some(capabilities) = capabilities else {
        return hints;
    };
    let client = db.lock().await.clone();
    let Some(client) = client else {
        return hints;
    };

    // The strongest read-only scope this connection supports, bounded by
    // PROBE_TIMEOUT_MS. Dropping this session — including when the outer 2s
    // timeout cancels the probe future mid-flight — spawns the teardown, so a
    // probe can never leak a transaction or a session-level statement_timeout.
    let readonly = match crate::readonly::ReadOnlySession::begin(
        &client,
        connection_id,
        capabilities,
        PROBE_TIMEOUT_MS,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            log::debug!("Pre-flight value probe could not open a read-only scope: {e}");
            return hints;
        }
    };
    for lit in literals {
        let candidates = probe_candidates(graph, lit);
        let Some(sql) = build_probe_sql(&candidates, lit) else {
            continue;
        };
        match client.execute(connection_id, &sql).await {
            Ok(res) => {
                if let Some(row) = res.rows.first() {
                    if let (Some(col), Some(val)) = (row[0].as_str(), row[1].as_str()) {
                        hints.push(format!(
                            "'{val}' is a stored value of {col} (matches '{lit}')"
                        ));
                    }
                }
            }
            Err(e) => {
                log::debug!("Value probe for '{lit}' skipped: {e}");
            }
        }
        if hints.len() >= MAX_VALUE_HINTS {
            break;
        }
    }
    // C7: close the scope before the caller's next query begins.
    readonly.close().await;
    hints
}

/// Run the full pre-flight for one question. Value hints at every tier;
/// cluster retrieval only when the schema was NOT fully injected (Push tier
/// already carries everything, so retrieval would be pure duplication).
/// `db` is the shared database handle for live probing of high-cardinality
/// literals that sample matching can't cover. `capabilities` decides whether
/// live probing runs at all — `None` skips it rather than probing unprotected.
pub async fn run_preflight(
    connection_id: Option<ConnectionId>,
    db: Option<&Arc<Mutex<Option<ConnectorClient>>>>,
    graph: Option<&SchemaGraph>,
    embedder: Option<&Embedder>,
    tier: &ContextTier,
    question: &str,
    capabilities: Option<&lucent_protocol::DriverCapabilities>,
) -> Option<String> {
    let graph = graph?;
    let start = std::time::Instant::now();

    let literals = extract_literals(question);
    let mut value_hints = match_values(graph, &literals);

    // Live probes for literals the samples couldn't ground (high-cardinality
    // lookups: ticket numbers, names, codes).
    if let (Some(db), Some(capabilities)) = (db, capabilities) {
        let covered: Vec<String> = value_hints.clone();
        let uncovered: Vec<String> = literals
            .iter()
            .filter(|l| !covered.iter().any(|h| h.contains(l.as_str())))
            .cloned()
            .collect();
        if !uncovered.is_empty() {
            // Bound probe time: even with 500ms per literal, 8 literals × 500ms = 4s.
            // If it exceeds 2s total, skip the rest — sample hints are still available.
            let probe_fut = probe_literals(
                connection_id.unwrap_or(ConnectionId(uuid::Uuid::nil())),
                db,
                graph,
                &uncovered,
                Some(capabilities),
            );
            match tokio::time::timeout(std::time::Duration::from_secs(2), probe_fut).await {
                Ok(h) => value_hints.extend(h),
                Err(_) => log::warn!("Pre-flight value probing timed out after 2s"),
            }
            value_hints.truncate(MAX_VALUE_HINTS);
        }
    }

    let cluster_section = if *tier == ContextTier::Push {
        None
    } else if let Some(embedder) = embedder {
        match embedder.embed_query(question).await {
            Ok(v) => {
                let (max_hops, max_cluster) =
                    crate::ai::tools::search_schema::scaled_bfs_bounds(graph.tables.len());
                let matched = Retriever::top_matches(graph, &v, PREFLIGHT_TOP_K);
                if matched.is_empty() {
                    None
                } else {
                    let retrieved =
                        Retriever::cluster_from_matches(graph, matched, max_hops, max_cluster);
                    Some(crate::ai::tools::search_schema::format_semantic_section(
                        &retrieved,
                    ))
                }
            }
            Err(e) => {
                log::warn!("Pre-flight embedding failed, continuing without cluster: {e}");
                None
            }
        }
    } else {
        None
    };

    let block = assemble_block(cluster_section, &value_hints);
    log::info!(
        "Pre-flight done in {:.0?}: {} literals, {} value hints, cluster={}",
        start.elapsed(),
        literals.len(),
        value_hints.len(),
        block
            .as_deref()
            .map(|b| b.contains("Semantic"))
            .unwrap_or(false),
    );
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::mschema::ContextTier;
    use crate::ai::schema_graph::{ColumnEntry, SchemaGraph, TableEntry};
    use std::collections::HashMap;

    fn graph_with_values() -> SchemaGraph {
        let columns = vec![ColumnEntry {
            id: 0,
            table_id: 0,
            schema: "bookings".into(),
            table: "airports".into(),
            name: "airport_code".into(),
            data_type: "character".into(),
            is_primary_key: false,
            sample_values: vec!["CAN".into(), "SZX".into(), "CDG".into()],
            fk_ref: None,
            embedding: vec![],
            doc_text: String::new(),
        }];
        SchemaGraph {
            tables: vec![TableEntry {
                id: 0,
                schema: "bookings".into(),
                name: "airports".into(),
                row_count_estimate: 104,
                partition_info: None,
            }],
            columns_by_table: HashMap::from([(0, vec![0])]),
            columns,
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at_unix: 0,
            tier: crate::ai::schema_graph::IndexingTier::MetadataOnly,
        }
    }

    fn graph_for_probing() -> SchemaGraph {
        let mk = |id: usize, table: &str, name: &str, dt: &str, pk: bool, samples: Vec<&str>| {
            ColumnEntry {
                id,
                table_id: 0,
                schema: "bookings".into(),
                table: table.into(),
                name: name.into(),
                data_type: dt.into(),
                is_primary_key: pk,
                sample_values: samples.into_iter().map(String::from).collect(),
                fk_ref: None,
                embedding: vec![],
                doc_text: String::new(),
            }
        };
        let columns = vec![
            mk(0, "tickets", "ticket_no", "character", true, vec![]), // PK, unsampled
            mk(
                1,
                "tickets",
                "passenger_name",
                "text",
                false,
                vec!["IVAN PETROV"],
            ),
            mk(
                2,
                "flights",
                "status",
                "text",
                false,
                vec!["Arrived", "Cancelled"],
            ),
            mk(3, "flights", "flight_id", "integer", true, vec![]), // non-text: never probed
        ];
        SchemaGraph {
            tables: vec![TableEntry {
                id: 0,
                schema: "bookings".into(),
                name: "tickets".into(),
                row_count_estimate: 0,
                partition_info: None,
            }],
            columns_by_table: HashMap::from([(0, vec![0, 1, 2, 3])]),
            columns,
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at_unix: 0,
            tier: crate::ai::schema_graph::IndexingTier::MetadataOnly,
        }
    }

    #[test]
    fn shape_of_maps_character_classes() {
        assert_eq!(shape_of("0005433348362"), "9999999999999");
        assert_eq!(shape_of("PG0072"), "AA9999");
        assert_eq!(shape_of("Bao'an"), "Aaa'aa");
    }

    #[test]
    fn probe_candidates_shape_matches_sampled_columns() {
        let g = graph_for_probing();
        let cands = probe_candidates(&g, "OLEG PETROV");
        assert!(
            cands.iter().any(|c| c.name == "passenger_name"),
            "sampled column with identical value shape must be a candidate"
        );
        assert!(
            !cands.iter().any(|c| c.name == "flight_id"),
            "non-text columns are never probed"
        );
    }

    #[test]
    fn probe_candidates_includes_unsampled_key_columns() {
        let g = graph_for_probing();
        let cands = probe_candidates(&g, "0005433348362");
        assert!(
            cands.iter().any(|c| c.name == "ticket_no"),
            "unsampled PK/FK text columns are probed — they are usually indexed"
        );
    }

    #[test]
    fn probe_sql_is_single_statement_with_case_variants() {
        let g = graph_for_probing();
        let cands = probe_candidates(&g, "0005433348362");
        let sql = build_probe_sql(&cands, "0005433348362").expect("candidates exist");
        assert!(!sql.contains(';'), "single statement only: {sql}");
        assert!(sql.contains("LIMIT 1"), "each probe stops at first hit");
        assert!(
            sql.contains("IN ("),
            "index-friendly equality, not ILIKE: {sql}"
        );
    }

    #[test]
    fn probe_sql_dedupes_case_variants_and_escapes_quotes() {
        let g = graph_for_probing();
        let cands = probe_candidates(&g, "OLEG PETROV");
        let sql = build_probe_sql(&cands, "O'LEG").expect("candidates exist");
        assert!(sql.contains("O''LEG"), "single quotes doubled: {sql}");
        assert!(!sql.contains("ILIKE"), "equality only");
    }

    #[tokio::test]
    async fn preflight_without_db_handle_behaves_as_before() {
        let g = graph_with_values();
        let out = run_preflight(
            None,
            None,
            Some(&g),
            None,
            &ContextTier::Push,
            "flights from CAN",
            None,
        )
        .await;
        assert!(out
            .expect("sample hint still fires")
            .contains("airport_code"));
    }

    #[test]
    fn extracts_quoted_and_all_caps_literals() {
        let lits = extract_literals("flights from CAN to 'Shenzhen Bao'");
        assert!(lits.contains(&"CAN".to_string()), "{lits:?}");
        assert!(
            lits.contains(&"Shenzhen Bao".to_string()),
            "quoted span kept whole: {lits:?}"
        );
    }

    #[test]
    fn ignores_plain_lowercase_words_and_question_stopwords() {
        assert!(extract_literals("what are the cheapest routes?").is_empty());
        assert!(
            extract_literals("Show me all flights").is_empty(),
            "sentence-leading stopwords like Show/All must not be treated as literals"
        );
    }

    #[test]
    fn extracts_capitalized_and_digit_bearing_tokens() {
        let lits = extract_literals("revenue for route PG0072 to Shenzhen");
        assert!(lits.contains(&"PG0072".to_string()));
        assert!(lits.contains(&"Shenzhen".to_string()));
    }

    #[test]
    fn value_match_reports_column_for_known_literal_case_insensitively() {
        let g = graph_with_values();
        let hints = match_values(&g, &["can".to_string()]);
        assert_eq!(hints.len(), 1);
        assert!(
            hints[0].contains("bookings.airports.airport_code"),
            "{hints:?}"
        );
        assert!(
            hints[0].contains("'CAN'"),
            "hint shows the STORED casing: {hints:?}"
        );
    }

    #[test]
    fn value_match_silent_for_unknown_literal() {
        let g = graph_with_values();
        assert!(match_values(&g, &["XYZZY".to_string()]).is_empty());
    }

    #[test]
    fn literal_matches_inside_longer_sample_value_by_containment() {
        let mut g = graph_with_values();
        g.columns[0].sample_values = vec!["Boeing 777-300ER".into()];
        let hints = match_values(&g, &["777-300ER".to_string()]);
        assert_eq!(
            hints.len(),
            1,
            "containment must ground jsonb-extracted values"
        );
        assert!(
            hints[0].contains("Boeing 777-300ER"),
            "hint shows the full stored value: {hints:?}"
        );
    }

    #[test]
    fn short_literals_do_not_containment_match() {
        let mut g = graph_with_values();
        g.columns[0].sample_values = vec!["CAN-2024-ARCHIVE".into()];
        let hints = match_values(&g, &["CAN".to_string()]);
        assert!(
            hints.iter().all(|h| !h.contains("ARCHIVE")),
            "3-char literals only exact-match — containment would be noise: {hints:?}"
        );
    }

    #[test]
    fn literal_exactly_at_containment_threshold_matches() {
        let mut g = graph_with_values();
        g.columns[0].sample_values = vec!["Boeing 777".into()];
        // CONTAINMENT_MIN_CHARS = 4; "777" is 3 chars, "Boei" is 4 chars
        let hints_short = match_values(&g, &["777".to_string()]);
        assert!(
            hints_short.is_empty(),
            "3-char literal must not containment-match: {hints_short:?}"
        );
        let hints_boundary = match_values(&g, &["Boei".to_string()]);
        assert!(
            hints_boundary.iter().any(|h| h.contains("Boeing 777")),
            "4-char literal must containment-match: {hints_boundary:?}"
        );
    }

    #[test]
    fn empty_literal_does_not_match_anything() {
        let g = graph_with_values();
        let hints = match_values(&g, &["".to_string()]);
        assert!(hints.is_empty(), "empty literal must not match any value");
    }

    #[test]
    fn two_literals_matching_same_column_produce_two_hints() {
        let mut g = graph_with_values();
        g.columns[0].sample_values = vec!["Boeing 777-300ER".into(), "Boeing 737-800".into()];
        let hints = match_values(&g, &["777-300ER".to_string(), "737-800".to_string()]);
        assert_eq!(
            hints.len(),
            2,
            "each matching literal must produce its own hint: {hints:?}"
        );
        assert!(hints[0].contains("777-300ER"));
        assert!(hints[1].contains("737-800"));
    }

    #[test]
    fn assemble_block_none_when_nothing_to_say() {
        assert!(assemble_block(None, &[]).is_none());
    }

    #[test]
    fn assemble_block_labels_itself_as_machine_retrieved() {
        let block = assemble_block(
            None,
            &["'CAN' is a stored value of bookings.airports.airport_code".into()],
        )
        .unwrap();
        assert!(
            block.contains("[Schema context — retrieved automatically"),
            "{block}"
        );
        assert!(block.contains("'CAN' is a stored value"));
    }

    #[tokio::test]
    async fn preflight_push_tier_returns_value_hints_without_embedder() {
        let g = graph_with_values();
        let out = run_preflight(
            None,
            None,
            Some(&g),
            None,
            &ContextTier::Push,
            "flights from CAN",
            None,
        )
        .await;
        let block = out.expect("value hint present");
        assert!(block.contains("airport_code"));
        assert!(
            !block.contains("Semantic Search"),
            "push tier never runs cluster retrieval"
        );
    }

    #[tokio::test]
    async fn preflight_none_without_graph() {
        assert!(
            run_preflight(None, None, None, None, &ContextTier::Pull, "anything", None)
                .await
                .is_none()
        );
    }
}
