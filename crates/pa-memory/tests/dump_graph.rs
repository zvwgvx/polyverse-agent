use anyhow::Result;

#[tokio::test]
async fn test_full_roundtrip() -> Result<()> {
    let graph = pa_memory::graph::CognitiveGraph::new("data/ryuuko_graph").await?;
    
    // Clean slate
    graph.db.query("DELETE attitudes_towards:ryuuko_roundtrip; DELETE illusion_of:roundtrip_ryuuko;").await?;
    
    // Step 1: First write
    println!("=== First write ===");
    let delta1 = pa_memory::graph::SocialDelta {
        delta_affinity: 0.15,
        delta_attachment: 0.05,
        delta_trust: 0.10,
        delta_safety: 0.08,
        delta_tension: -0.02,
    };
    graph.update_social_graph("roundtrip", delta1).await?;
    
    let (att1, _) = graph.get_social_context("roundtrip").await?;
    println!("After 1st: Affinity={:.4}, Attachment={:.4}, Trust={:.4}, Safety={:.4}, Tension={:.4}",
        att1.affinity, att1.attachment, att1.trust, att1.safety, att1.tension);
    assert!((att1.affinity - 0.15).abs() < 0.001, "Expected 0.15, got {}", att1.affinity);
    
    // Step 2: Accumulate
    println!("\n=== Second write (accumulate) ===");
    let delta2 = pa_memory::graph::SocialDelta {
        delta_affinity: 0.10,
        delta_attachment: 0.10,
        delta_trust: 0.0,
        delta_safety: 0.0,
        delta_tension: 0.0,
    };
    graph.update_social_graph("roundtrip", delta2).await?;
    
    let (att2, _) = graph.get_social_context("roundtrip").await?;
    println!("After 2nd: Affinity={:.4}, Attachment={:.4}, Trust={:.4}, Safety={:.4}, Tension={:.4}",
        att2.affinity, att2.attachment, att2.trust, att2.safety, att2.tension);
    assert!((att2.affinity - 0.25).abs() < 0.001, "Expected 0.25, got {}", att2.affinity);
    assert!((att2.attachment - 0.15).abs() < 0.001, "Expected 0.15, got {}", att2.attachment);
    assert!((att2.trust - 0.10).abs() < 0.001, "Trust should stay at 0.10, got {}", att2.trust);
    
    println!("\n✅ All assertions passed! Delta accumulation works!");
    
    // Clean up
    graph.db.query("DELETE attitudes_towards:ryuuko_roundtrip;").await?;
    
    Ok(())
}
