pub mod activity;
pub mod agent;
pub mod config;
pub mod context;
pub mod embed;
pub mod events;
pub mod guard;
pub mod mschema;
pub mod preflight;
pub mod provider;
pub mod providers;
pub mod rerank;
pub mod retrieval;
pub mod schema_graph;
pub mod sql_lint;
pub mod tools;

#[cfg(any(test, feature = "evals"))]
pub mod evals;

#[cfg(feature = "integration-tests")]
pub mod integration_test;

pub use config::AiConfig;
pub use events::AiEvent;
pub use provider::{LlmProvider, LucentAgent};

/// Truncate `s` to at most `max` bytes, cutting only on a UTF-8 char boundary.
pub fn truncate_utf8(s: &str, max: usize) -> &str {
    let idx = s.floor_char_boundary(max.min(s.len()));
    &s[..idx]
}

#[cfg(test)]
mod truncation_tests {
    use super::truncate_utf8;

    #[test]
    fn truncate_utf8_never_splits_a_multibyte_char() {
        let s = "é".repeat(6000);
        let cut = truncate_utf8(&s, 5000);
        assert!(cut.len() <= 5000);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());

        let wide = "中".repeat(6000);
        let cut = truncate_utf8(&wide, 5000);
        assert!(cut.len() <= 5000);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
    }
}
