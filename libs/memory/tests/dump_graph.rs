use anyhow::Result;
use memory::graph::{CognitiveGraph, SocialDelta};

fn approx_eq(left: f32, right: f32) {
    assert!((left - right).abs() < 0.001, "left={left} right={right}");
}

fn unique_user_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    format!("{prefix}_{nanos}")
}

#[tokio::test]
async fn social_graph_accumulates_updates_across_writes() -> Result<()> {
    let graph = CognitiveGraph::new("memory").await?;
    let user_id = unique_user_id("roundtrip");

    graph
        .update_social_graph(
            &user_id,
            SocialDelta {
                delta_affinity: 0.15,
                delta_attachment: 0.05,
                delta_trust: 0.10,
                delta_safety: 0.08,
                delta_tension: -0.02,
            },
        )
        .await?;

    let (first, _) = graph.get_social_context(&user_id).await?;
    approx_eq(first.affinity, 0.15);
    approx_eq(first.attachment, 0.05);
    approx_eq(first.trust, 0.10);
    approx_eq(first.safety, 0.08);
    approx_eq(first.tension, -0.02);

    graph
        .update_social_graph(
            &user_id,
            SocialDelta {
                delta_affinity: 0.10,
                delta_attachment: 0.10,
                ..Default::default()
            },
        )
        .await?;

    let (second, _) = graph.get_social_context(&user_id).await?;
    approx_eq(second.affinity, 0.25);
    approx_eq(second.attachment, 0.15);
    approx_eq(second.trust, 0.10);
    approx_eq(second.safety, 0.08);
    approx_eq(second.tension, -0.02);
    Ok(())
}

#[tokio::test]
async fn social_graph_clamps_large_deltas_before_persisting() -> Result<()> {
    let graph = CognitiveGraph::new("memory").await?;
    let user_id = unique_user_id("clamped");

    graph
        .update_social_graph(
            &user_id,
            SocialDelta {
                delta_affinity: 5.0,
                delta_attachment: -5.0,
                delta_trust: 0.31,
                delta_safety: -0.31,
                delta_tension: 1.20,
            },
        )
        .await?;

    let (attitudes, _) = graph.get_social_context(&user_id).await?;
    approx_eq(attitudes.affinity, 0.30);
    approx_eq(attitudes.attachment, -0.30);
    approx_eq(attitudes.trust, 0.30);
    approx_eq(attitudes.safety, -0.30);
    approx_eq(attitudes.tension, 0.30);
    Ok(())
}

#[tokio::test]
async fn get_social_context_defaults_to_zero_for_unknown_user() -> Result<()> {
    let graph = CognitiveGraph::new("memory").await?;

    let (attitudes, illusion) = graph.get_social_context("unknown-user").await?;
    approx_eq(attitudes.affinity, 0.0);
    approx_eq(attitudes.attachment, 0.0);
    approx_eq(attitudes.trust, 0.0);
    approx_eq(attitudes.safety, 0.0);
    approx_eq(attitudes.tension, 0.0);
    approx_eq(illusion.affinity, 0.0);
    approx_eq(illusion.attachment, 0.0);
    approx_eq(illusion.trust, 0.0);
    approx_eq(illusion.safety, 0.0);
    approx_eq(illusion.tension, 0.0);
    Ok(())
}

#[tokio::test]
async fn get_or_project_social_tree_snapshot_projects_when_missing_and_reads_afterward() -> Result<()> {
    let graph = CognitiveGraph::new("memory").await?;
    let user_id = unique_user_id("cached_user");

    let first = graph
        .get_or_project_social_tree_snapshot(&user_id, 0.10)
        .await?;

    let stored = graph.get_social_tree_snapshot(&user_id).await;
    let second = graph
        .get_or_project_social_tree_snapshot(&user_id, 0.95)
        .await?;

    assert_eq!(first.user_id, user_id);
    assert!(stored.is_ok(), "projected snapshot should be readable immediately");
    let stored = stored?;
    assert_eq!(first.derived_summaries.familiarity_bucket, "new");
    approx_eq(first.relationship_core.familiarity, 0.10);

    assert_eq!(stored.user_id, first.user_id);
    assert_eq!(stored.meta.schema_version, "v1");
    assert!(!stored.meta.updated_at.is_empty());

    assert_eq!(second.user_id, first.user_id);
    assert_eq!(second.meta.schema_version, "v1");
    assert!(!second.meta.updated_at.is_empty());
    approx_eq(second.relationship_core.familiarity, stored.relationship_core.familiarity);
    assert_eq!(
        second.derived_summaries.familiarity_bucket,
        stored.derived_summaries.familiarity_bucket
    );
    Ok(())
}

#[tokio::test]
async fn snapshot_relationship_graph_includes_social_and_illusion_edges() -> Result<()> {
    let graph = CognitiveGraph::new("memory").await?;
    let self_node_id = graph.self_node_id().to_string();
    let alice = unique_user_id("alice");
    let alice_node_id = format!("person:{alice}");

    graph
        .update_social_graph(
            &alice,
            SocialDelta {
                delta_affinity: 0.20,
                ..Default::default()
            },
        )
        .await?;
    graph
        .update_illusion_graph(
            &alice,
            SocialDelta {
                delta_trust: 0.15,
                ..Default::default()
            },
        )
        .await?;

    let snapshot = graph.snapshot_relationship_graph().await?;

    assert_eq!(snapshot.self_node_id, self_node_id);
    assert!(snapshot
        .nodes
        .iter()
        .any(|node| node.id == snapshot.self_node_id && node.kind == "agent"));
    assert!(snapshot
        .nodes
        .iter()
        .any(|node| node.id == alice_node_id && node.kind == "person"));

    assert!(snapshot.edges.iter().any(|edge| {
        edge.kind == "social"
            && edge.source == snapshot.self_node_id
            && edge.target == alice_node_id
            && edge.affinity.map(|v| v > 0.0).unwrap_or(false)
    }));
    assert!(snapshot.edges.iter().any(|edge| {
        edge.kind == "illusion"
            && edge.source == alice_node_id
            && edge.target == snapshot.self_node_id
            && edge.trust.map(|v| v > 0.0).unwrap_or(false)
    }));
    Ok(())
}

#[tokio::test]
async fn update_observed_dynamic_writes_tension_edge() -> Result<()> {
    let graph = CognitiveGraph::new("memory").await?;
    let alice = unique_user_id("observer_a");
    let bob = unique_user_id("observer_b");
    let edge_id = format!("{alice}_{bob}");

    graph.update_observed_dynamic(&alice, &bob, 0.25).await?;

    let mut response = graph
        .db
        .query(format!(
            "SELECT string::concat('', in) AS source, string::concat('', out) AS target, tension FROM interacts_with:`{edge_id}` LIMIT 1;"
        ))
        .await?;
    let rows: Vec<serde_json::Value> = response.take(0).unwrap_or_default();
    let row = rows.first().expect("observed dynamic row should exist");

    let expected_source = format!("person:{alice}");
    let expected_target = format!("person:{bob}");
    assert_eq!(row.get("source").and_then(|v| v.as_str()), Some(expected_source.as_str()));
    assert_eq!(row.get("target").and_then(|v| v.as_str()), Some(expected_target.as_str()));
    assert_eq!(row.get("tension").and_then(|v| v.as_f64()), Some(0.25));
    Ok(())
}
