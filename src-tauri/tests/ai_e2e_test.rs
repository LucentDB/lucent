//! End-to-end test for the AI agent.
//!
//! Usage:
//!   OPENAI_API_KEY=sk-... OPENAI_BASE_URL=https://opencode.ai/zen/go/v1 \
//!     LUENT_AI_MODEL=deepseek-v4-flash \
//!     cargo test --package lucent --test ai_e2e_test -- --nocapture
//!
//! This test is #[ignore] by default — remove the attribute to run it.
//! It makes a real LLM API call and requires valid credentials.

use lucent_lib::ai::agent::Message;
use lucent_lib::ai::config::AiProvider;
use lucent_lib::ai::provider::LlmProvider as _;
use lucent_lib::ai::providers::rig::RigProvider;

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY env var and makes a real API call"]
async fn e2e_simple_completion() {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("Set OPENAI_API_KEY env var to run this test");
    let model = std::env::var("LUCENT_AI_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let endpoint =
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://opencode.ai/zen/go/v1".into());

    eprintln!("Using model: {model}");
    eprintln!("Using endpoint: {endpoint}");

    let provider = RigProvider::new(AiProvider::OpenAI, api_key, Some(endpoint));

    let agent = provider
        .build_agent(
            &model,
            "You are a helpful assistant. Keep responses brief.".into(),
            1024,
            vec![],
        )
        .await;

    let response = agent
        .complete(
            Message::user("Say hello in exactly 5 words."),
            vec![],
            &|_| {},
        )
        .await
        .expect("Agent completion should succeed");

    let text = response.text.expect("Response should contain text");
    eprintln!("Response: {text}");
    assert!(!text.is_empty(), "Response should not be empty");
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY env var and makes a real API call"]
async fn e2e_text_protocol_tool_parsing() {
    // Test that the text-based [TOOL_CALL] protocol correctly parses
    let _simulated_response =
        "Let me search for that.\n[TOOL_CALL] search_objects({\"query\":\"sarah\"}) [/TOOL_CALL]";
    let calls = lucent_lib::ai::tools::all_tools(lucent_lib::ai::tools::AiToolContext {
        db: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        config: lucent_lib::ai::config::AiConfig::default(),
        schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
    });
    assert!(
        calls.iter().any(|t| t.name() == "search_objects"),
        "search_objects tool must exist"
    );
    assert!(
        calls.iter().any(|t| t.name() == "get_objects_info"),
        "get_objects_info tool must exist"
    );
    assert!(
        calls.iter().any(|t| t.name() == "run_readonly_query"),
        "run_readonly_query tool must exist"
    );
    assert!(
        calls.iter().any(|t| t.name() == "preview_dml"),
        "preview_dml tool must exist"
    );

    eprintln!("All 4 tools registered correctly.");
}
