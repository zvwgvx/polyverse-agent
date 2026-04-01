use pa_memory::graph::CognitiveGraph;
use pa_mcp::{McpConfig, McpWorker};

#[tokio::test]
async fn mcp_worker_constructs_with_default_registry() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");

    let worker = McpWorker::new(McpConfig::default(), graph);
    let tools = worker.registry().list();

    assert_eq!(tools.len(), 2);
    assert!(tools.iter().any(|t| t.name == "social.get_affect_context"));
    assert!(tools.iter().any(|t| t.name == "social.get_dialogue_summary"));
    assert!(tools.iter().all(|t| t.read_only));
}
