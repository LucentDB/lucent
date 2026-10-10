use serde_json::json;

use super::{AiToolContext, ToolError, ToolOutput};
use lucent_protocol::{CatalogRequest, CatalogResult, FactSection, FactValue};

/// Pure renderer: one `name: …` line per section, in the driver's order.
pub(crate) fn report_for(engine: &str, facts: &[FactSection]) -> String {
    let mut out = format!("Connection facts ({engine}):\n");
    for f in facts {
        let value = match &f.value {
            FactValue::Rows(r) if r.is_empty() => "(none)".to_string(),
            FactValue::Rows(r) => r.join("; "),
            FactValue::Unavailable(e) => format!("unavailable ({e})"),
            FactValue::NotSupported => "not supported on this engine".to_string(),
        };
        out.push_str(&format!("{}: {value}\n", f.name));
    }
    out
}

#[derive(Clone)]
pub struct InspectConnection {
    ctx: AiToolContext,
}

impl InspectConnection {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }

    pub fn description(&self) -> String {
        "Inspect live connection facts: current user and role memberships, timezone, \
         search_path, statement_timeout, extensions (installed and available), and schema \
         sizes. Use this instead of querying system catalogs directly."
            .into()
    }

    pub fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    pub async fn call(
        &self,
        _args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let capabilities = self
            .ctx
            .capabilities
            .as_ref()
            .ok_or(ToolError::NotConnected)?;
        let conn_id = self.ctx.connection_id.ok_or(ToolError::NotConnected)?;
        let client = self
            .ctx
            .db
            .lock()
            .await
            .clone()
            .ok_or(ToolError::NotConnected)?;
        let wait_ms = self
            .ctx
            .config
            .ai_query_timeout_secs
            .saturating_mul(1000)
            .max(5_000);
        // Ordered with the AI's other statement sequences (Task 3A): on Postgres
        // a failed catalog read inside another task's open transaction would
        // abort that transaction.
        let _sequence = client
            .lock_sequence(conn_id, wait_ms)
            .await
            .map_err(ToolError::Execution)?;
        let facts = match client
            .catalog(conn_id, CatalogRequest::ConnectionFacts)
            .await
            .map_err(ToolError::Execution)?
        {
            CatalogResult::ConnectionFacts(f) => f,
            other => {
                return Err(ToolError::Execution(format!(
                    "unexpected catalog reply: {other:?}"
                )))
            }
        };
        Ok(ToolOutput::Text {
            content: report_for(&capabilities.display_name, &facts),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_renders_rows_failures_and_unsupported_sections() {
        let r = report_for(
            "PostgreSQL",
            &[
                FactSection {
                    name: "identity".into(),
                    value: FactValue::Rows(vec!["user=postgres, database=demo".into()]),
                },
                FactSection {
                    name: "extensions".into(),
                    value: FactValue::Unavailable(
                        "permission denied for table pg_extension".into(),
                    ),
                },
                FactSection {
                    name: "roles".into(),
                    value: FactValue::Rows(vec![]),
                },
                FactSection {
                    name: "search_path".into(),
                    value: FactValue::NotSupported,
                },
            ],
        );
        assert!(r.starts_with("Connection facts (PostgreSQL):"), "{r}");
        assert!(r.contains("identity: user=postgres, database=demo"), "{r}");
        assert!(
            r.contains("extensions: unavailable (permission denied"),
            "{r}"
        );
        assert!(r.contains("roles: (none)"), "{r}");
        assert!(
            r.contains("search_path: not supported on this engine"),
            "{r}"
        );
    }
}
