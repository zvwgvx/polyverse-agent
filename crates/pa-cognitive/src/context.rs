use std::sync::Arc;
use pa_memory::{
    episodic::EpisodicStore,
    embedder::MemoryEmbedder,
    graph::CognitiveGraph,
};

/// A shared cognitive context built for both System 1 and System 2.
pub struct SharedCognitiveContext {
    pub memory_text: Option<String>,
    pub social_text: String,
    pub time_and_history_text: String,
}

pub async fn build_shared_cognitive_context(
    history: &[(String, String, String)],
    episodic: Option<&Arc<EpisodicStore>>,
    embedder: Option<&Arc<MemoryEmbedder>>,
    graph: &CognitiveGraph,
    current_username: &str,
    new_message: &str,
) -> SharedCognitiveContext {
    // 1. RAG Memory (Hồi Hải Mã)
    let mut memory_text = None;
    if let (Some(ep), Some(emb)) = (episodic, embedder) {
        let recent_context = history.iter()
            .rev()
            .take(2)
            .map(|(_, _, content)| content.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        
        let search_query = if recent_context.is_empty() {
            new_message.to_string()
        } else {
            format!("{} | {}", recent_context, new_message)
        };

        if let Ok(query_vec) = emb.embed_single(search_query).await {
            if let Ok(events) = ep.search(&query_vec, 3, 0.5).await {
                if !events.is_empty() {
                    let mut text = String::from("### TỰ SUY NGHĨ (KÝ ỨC CŨ): Những thông tin sau có thể liên quan đến ngữ cảnh cuộc trò chuyện. Hãy linh hoạt sử dụng nếu thấy cần thiết (KHÔNG nhắc lại nguyên văn):\n");
                    for ev in events {
                        let date_str = chrono::DateTime::from_timestamp(ev.timestamp, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                            .unwrap_or_else(|| "Unknown date".to_string());
                        text.push_str(&format!("- [Vào {}]: {}\n", date_str, ev.content));
                    }
                    memory_text = Some(text);
                }
            }
        }
    }

    // 2. Context Depth and Social Weight (Nhận thức Cảm xúc & Bối cảnh)
    // Primary: graph weights (naturally slow, decayed — best measure of relationship depth)
    // Secondary: small LanceDB memory hint (shared memories exist)
    let lancedb_count = if let Some(ep) = episodic {
        ep.count_user_chunks(current_username).await.unwrap_or(0_usize)
    } else {
        0_usize
    };
    let memory_hint = (lancedb_count as f32 / 200.0).min(0.15);

    let mut social_text = format!(
        "### TRẠNG THÁI CẢM XÚC, QUAN HỆ & BỐI CẢNH VỚI {}:\n", 
        current_username
    );
    
    if let Ok((attitudes, illusion)) = graph.get_social_context(current_username).await {
        // context_depth derived from accumulated graph weights + small memory bonus
        let graph_depth = (attitudes.affinity.abs() + attitudes.attachment.abs() 
            + attitudes.trust.abs() + attitudes.safety.abs()) / 4.0;
        let context_depth = (graph_depth + memory_hint).min(1.0);

        social_text.push_str(&format!(
            "- Mức độ Hảo cảm (Affinity): {:.6} (-1 ghét tởm, 1 yêu quý)\n",
            attitudes.affinity
        ));
        social_text.push_str(&format!(
            "- Độ thân thiết/Bám dính (Attachment): {:.6} (-1 né tránh, 1 bám lấy)\n",
            attitudes.attachment
        ));
        social_text.push_str(&format!(
            "- Niềm tin (Trust): {:.6} (-1 nghi ngờ, 1 tin tưởng tuyệt đối)\n",
            attitudes.trust
        ));
        social_text.push_str(&format!(
            "- Cảm giác An toàn (Safety): {:.6} (-1 sợ hãi/deft, 1 an tâm)\n",
            attitudes.safety
        ));
        social_text.push_str(&format!(
            "- Độ căng thẳng (Tension): {:.6} (0 thoải mái, 1 áp lực/tức giận)\n",
            attitudes.tension
        ));
        social_text.push_str(&format!(
            "- Độ sâu Bối cảnh (Context Depth): {:.6} (0.0 người lạ mới quen, 1.0 bạn tâm giao lâu năm)\n\n",
            context_depth
        ));
        
        social_text.push_str(&format!(
            "Lưu ý: Bạn (Ryuuko) đang có ảo tưởng rằng {} cảm nhận về bạn như sau:\n",
            current_username
        ));
        social_text.push_str(&format!(
            "- Họ thích/ghét bạn (Affinity): {:.6} | Thân thiết (Attachment): {:.6} | Tin tưởng (Trust): {:.6} | An toàn (Safety): {:.6} | Căng thẳng (Tension): {:.6}\n",
            illusion.affinity, illusion.attachment, illusion.trust, illusion.safety, illusion.tension
        ));
    } else {
        social_text.push_str("- Mức độ Hảo cảm (Affinity): 0.000000 (-1 ghét tởm, 1 yêu quý)\n");
        social_text.push_str("- Độ thân thiết/Bám dính (Attachment): 0.000000 (-1 né tránh, 1 bám lấy)\n");
        social_text.push_str("- Niềm tin (Trust): 0.000000 (-1 nghi ngờ, 1 tin tưởng tuyệt đối)\n");
        social_text.push_str("- Cảm giác An toàn (Safety): 0.000000 (-1 sợ hãi/deft, 1 an tâm)\n");
        social_text.push_str("- Độ căng thẳng (Tension): 0.000000 (0 thoải mái, 1 áp lực/tức giận)\n");
        social_text.push_str(&format!(
            "- Độ sâu Bối cảnh (Context Depth): {:.6} (0.0 người lạ mới quen, 1.0 bạn tâm giao lâu năm)\n\n",
            memory_hint
        ));
        
        social_text.push_str(&format!(
            "Lưu ý: Bạn (Ryuuko) đang có ảo tưởng rằng {} cảm nhận về bạn như sau:\n",
            current_username
        ));
        social_text.push_str("- Họ thích/ghét bạn (Affinity): 0.000000 | Thân thiết (Attachment): 0.000000 | Tin tưởng (Trust): 0.000000 | An toàn (Safety): 0.000000 | Căng thẳng (Tension): 0.000000\n");
    }

    // 3. Current Time and Short-Term Context summary
    let now = chrono::Utc::now();
    let sg_time = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
    let vn_time = now.with_timezone(&chrono::FixedOffset::east_opt(7 * 3600).unwrap());
    let mut time_and_history_text = format!(
        "[NOW]: UTC: {} | Ryuuko(GMT+8): {} | User(GMT+7): {}\n",
        now.format("%d/%m/%Y %H:%M:%S"),
        sg_time.format("%d/%m/%Y %H:%M:%S"),
        vn_time.format("%d/%m/%Y %H:%M:%S")
    );

    if history.is_empty() {
        if lancedb_count > 0 {
            time_and_history_text.push_str(&format!(
                "[context: đây là tin nhắn đầu tiên của phiên trò chuyện mới. người dùng ({}) là người quen cũ, hãy dựa vào đồ thị bối cảnh phía trên để cư xử phù hợp.]",
                current_username
            ));
        } else {
            time_and_history_text.push_str(&format!(
                "[context: đây là tin nhắn đầu tiên từ {}. mày chưa biết người này — đây là người lạ. Hãy cẩn trọng.]",
                current_username
            ));
        }
    } else {
        let mut users = std::collections::HashSet::new();
        for (role, username, _) in history {
            if role == "user" {
                users.insert(username.as_str());
            }
        }
        let users_vec: Vec<&str> = users.into_iter().collect();
        time_and_history_text.push_str(&format!(
            "[context: đã có {} tin nhắn trước đó trong cuộc hội thoại này. người tham gia: {}. hãy trả lời dựa trên mạch hội thoại ở trên.]",
            history.len(),
            users_vec.join(", ")
        ));
    }

    SharedCognitiveContext {
        memory_text,
        social_text,
        time_and_history_text,
    }
}
