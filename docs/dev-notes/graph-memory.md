# Kiến Trúc Cốt Lõi: Đồ Thị Bất Biến & Nhận Thức Động (Immutable Graph & Dynamic Activation)

**Phiên bản:** 4.2 (Fix degree direction, parent_activation propagation, subgraph LIMIT, upsert race condition, UTF-8 token budget)  
**Mục tiêu:** Xây dựng hệ thống Graph Memory bất biến (Immutable), tính toán Activation suy giảm theo thời gian/độ sâu/bậc ngay tại RAM (Rust), xử lý triệt để từ trái nghĩa mà không bị ảo giác Embedding, và đảm bảo Latency Budget < 50ms.

---

## 1. Tổng Quan Luồng Xử Lý (The Full Pipeline)

```text
[User Message]
     │
     ▼ (1. Semantic Match)
[fastembed] ──> Vector Search ──> [Seed Nodes]
     │
     ▼ (2. Kéo Subgraph về RAM — 2 rounds)
[SurrealDB] ──> depth=1 nodes ──> depth=2 nodes ──> [Raw Subgraph]
     │
     ▼ (3. Hồi tưởng nhanh — Xử lý 100% trên RAM Rust)
[Spreading Activation Engine]
     ├── Temporal Decay      (Lãng quên theo thời gian)
     ├── Degree Penalty      (Giảm nhiễu Hub Nodes — tính từ subgraph)
     ├── IS_ALIAS_OF Pass    (Identity traversal, không decay)
     └── Context Packing     (Ép token < 800) ──> [Top-K Nodes]
     │
     ▼ (4. Prompt Assembly — Memory Shadowing)
[build_system_prompt()] ──> Graph context + Short-term (timestamp tự nhiên)
     │
     ▼ (5. Background Extraction — Bất đồng bộ)
[KnowledgeGraphWorker] ──> SLM/LLM trích xuất [Entity] -> [Rel] -> [Entity]
     │
     ▼ (6. Immutable Upsert)
[ACID Transaction] ──> Prefix+Semantic conflict detect, IS_ALIAS_OF (không gộp vật lý) ──> SurrealDB
```

---

## 2. Thiết Kế Lược Đồ (Immutable Graph Schema)

### Entity (Thực thể)
```rust
struct EntityNode {
    id: String,                       // SurrealDB Record ID (entity:uuid)
    name: String,                     // Tên hiển thị ("Đà Lạt", "Thành phố sương mù")
    type_kind: String,                // Location | Person | Concept | Preference
    description: String,
    embedding: Vec<f32>,              // Vector ngữ nghĩa của description
    created_at: DateTime<Utc>,
    last_activated_at: DateTime<Utc>, // Dùng cho Temporal Decay
    activation_count: u32,            // Nhắc nhiều = Core memory
}
```

### Relation (Mối quan hệ — Edge)
```rust
struct RelatesToEdge {
    in_node: RecordId,
    out_node: RecordId,
    relation_type: String,  // THICH | KHONG_THICH | MUON_DI | IS_ALIAS_OF | ...
    weight: f32,            // 0.0 -> 1.0
    confidence: f32,
    created_at: DateTime<Utc>,
    source_session_id: String,
}
```

---

## 3. Giai Đoạn Đọc: Spreading Activation Động (Rust-Side)

### Bước 3.1: Kéo Subgraph về RAM — 2 Rounds (Fix depth=2)

v4.0 chỉ fetch 1 hop. v4.1 dùng 2 round query song song:

```rust
async fn fetch_subgraph(
    db: &Surreal<Client>,
    seed_ids: &[RecordId],
) -> Result<RawSubgraph> {
    // Round 1: depth=1 — lấy tất cả edges + nodes liền kề seed
    // LIMIT 50: mỗi seed tối đa 50 edge, tránh graph bùng nổ ngay từ đầu
    let depth1: Vec<SubgraphRow> = db
        .query("SELECT in, out, relation_type, weight, confidence, created_at
                FROM relates_to
                WHERE in IN $seeds
                LIMIT 50
                FETCH out")
        .bind(("seeds", seed_ids))
        .await?
        .take(0)?;

    let depth1_node_ids: Vec<RecordId> = depth1
        .iter()
        .map(|r| r.out.clone())
        .collect();

    // Round 2: depth=2 — lấy edges từ các node depth=1
    // LIMIT 200: hard cap — depth1 có thể trả 50 node x nhiều edge, phải giới hạn
    // để tránh memory spike khi graph lớn
    let depth2: Vec<SubgraphRow> = db
        .query("SELECT in, out, relation_type, weight, confidence, created_at
                FROM relates_to
                WHERE in IN $d1_ids
                LIMIT 200
                FETCH out")
        .bind(("d1_ids", &depth1_node_ids))
        .await?
        .take(0)?;

    Ok(RawSubgraph { depth1, depth2 })
}
```

2 round query đơn giản, mỗi round là flat SELECT — không có lồng nhau, không có `$d1.activation` undefined.
Cả 2 round đều có LIMIT cứng để tránh memory spike khi Hub Node có hàng trăm edge.

---

### Bước 3.2: Tính Degree từ Subgraph (Fix node_degree — đếm out-degree)

Không cần thay schema. Tính từ edges đã có sẵn trên RAM.
Degree Penalty nhắm vào node **nguồn** đang lan truyền (node có nhiều edge đi ra),
vì vậy phải đếm `in_node` (out-degree của nguồn), không phải `out` (in-degree của đích):

```rust
fn build_degree_map(all_edges: &[SubgraphRow]) -> HashMap<RecordId, u32> {
    // Đếm out-degree: node "in" xuất hiện bao nhiêu lần với vai trò nguồn
    // Degree cao = Hub Node = cần penalty để không tràn activation ra mọi hướng
    all_edges.iter().fold(HashMap::new(), |mut map, edge| {
        *map.entry(edge.in_node.clone()).or_insert(0) += 1;
        map
    })
}
```

---

### Bước 3.3: Tính Năng Lượng (Temporal + Degree — lan truyền từ parent)

Hàm nhận `parent_activation` thay vì `seed_score`. Depth Decay đã được "nhúng" vào
`parent_activation` tích lũy từ bước trước — không tính lại từ seed gốc để tránh bỏ qua
weight của edge cha:

```rust
fn calculate_activation(
    parent_activation: f32,           // Năng lượng của node cha (không phải seed gốc)
    edge: &RelatesToEdge,
    entity: &EntityNode,
    degree_map: &HashMap<RecordId, u32>,
) -> f32 {
    // 1. Temporal Decay: e^(-0.01 * days)
    //    days=0   → 1.00  (hôm nay)
    //    days=30  → 0.74  (1 tháng)
    //    days=90  → 0.41  (3 tháng)
    //    days=180 → 0.17  (6 tháng)
    let days_since = (Utc::now() - entity.last_activated_at).num_days() as f32;
    let time_decay = (-0.01 * days_since).exp();

    // 2. Degree Penalty: chống Hub Node tràn ngập context
    //    out-degree=1   → penalty=1.00
    //    out-degree=9   → penalty=1.00  (log10(10)=1)
    //    out-degree=99  → penalty=0.50  (log10(100)=2)
    //    out-degree=999 → penalty=0.33  (log10(1000)=3)
    let degree = degree_map.get(&edge.in_node).copied().unwrap_or(1) as f32;
    let degree_penalty = 1.0 / (degree + 1.0).log10().max(1.0);

    // Lan truyền từ parent: depth_decay đã nằm trong parent_activation rồi
    parent_activation * edge.weight * time_decay * degree_penalty
}
```

---

### Bước 3.4: IS_ALIAS_OF Traversal + Depth Decay tường minh

Depth Decay (0.5 mỗi bước) được nhân trực tiếp tại điểm khởi động từ seed,
rồi truyền xuống dưới dạng `parent_activation`. IS_ALIAS_OF là identity — không decay thêm:

```rust
fn traverse_edge(
    edge: &RelatesToEdge,
    parent_activation: f32,           // Năng lượng tích lũy của node cha
    entity: &EntityNode,
    degree_map: &HashMap<RecordId, u32>,
) -> Option<f32> {
    match edge.relation_type.as_str() {
        // Identity relation: truyền nguyên activation của cha, không decay thêm
        // "Đà Lạt" IS_ALIAS_OF "Thành phố sương mù" → cùng activation
        "IS_ALIAS_OF" => Some(parent_activation),

        // Tất cả relation khác: nhân thêm depth decay 0.5, temporal decay, degree penalty
        _ => {
            // Depth decay được áp dụng tại đây (mỗi hop nhân 0.5)
            let decayed_parent = parent_activation * 0.5;
            let activation = calculate_activation(
                decayed_parent, edge, entity, degree_map
            );
            if activation > CUTOFF_THRESHOLD {
                Some(activation)
            } else {
                None // Cắt tỉa — không lan truyền tiếp
            }
        }
    }
}

const CUTOFF_THRESHOLD: f32 = 0.1;
```

---

### Bước 3.5: Context Packing (Chống Tràn Token)

```rust
fn pack_context(activated_nodes: &[(ActivatedNode, f32)]) -> String {
    // Sort by activation score giảm dần
    let mut sorted = activated_nodes.to_vec();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    let mut output = String::new();
    let mut token_budget = 800usize;

    for (node, score) in &sorted {
        let line = if node.depth == 0 {
            // Seed nodes: full description
            format!("[{}] {}: {}\n", node.entity.type_kind, node.entity.name, node.entity.description)
        } else {
            // Distant nodes: compact relation chain
            format!("{} -[{}]-> {}\n", node.parent_name, node.relation_type, node.entity.name)
        };

        let estimated_tokens = line.chars().count() / 2; // chars() không phải len() (bytes)
        // Dùng chars() vì tiếng Việt: 1 ký tự UTF-8 = 2-4 bytes nhưng ~0.5-1 token
        // len() / 4 sai hoàn toàn với văn bản không phải ASCII
        if estimated_tokens > token_budget {
            break;
        }
        token_budget -= estimated_tokens;
        output.push_str(&line);
    }

    output
}
```

---

### Bước 3.6: Latency Budget — Parallel Fetch + Timeout

```rust
async fn retrieve_cognitive_context(
    db: &Surreal<Client>,
    session_id: &str,
    embedding: &[f32],
) -> CognitiveContext {
    let timeout = Duration::from_millis(45);

    let (graph_result, short_term) = tokio::join!(
        // Path A: Vector search + Spreading Activation
        tokio::time::timeout(timeout, async {
            let seeds = vector_search(db, embedding).await?;
            if seeds.is_empty() {
                return Ok(vec![]);
            }
            let subgraph = fetch_subgraph(db, &seeds).await?;
            Ok(run_spreading_activation(subgraph, &seeds))
        }),
        // Path B: Short-term fallback — luôn chạy song song
        get_recent_session_nodes(db, session_id, 5)
    );

    let graph_nodes = graph_result
        .unwrap_or_default()  // timeout → dùng empty
        .unwrap_or_default();

    CognitiveContext { graph_nodes, short_term }
}
```

---

## 4. Prompt Assembly — Memory Shadowing (Fix conflict strategy)

Không cần conflict detection thủ công. LLM tự ưu tiên đúng dựa trên thứ tự và timestamp trong prompt:

```rust
fn build_system_prompt(ctx: &CognitiveContext) -> String {
    let graph_section = if ctx.graph_nodes.is_empty() {
        String::from("(Chưa có ký ức dài hạn)")
    } else {
        pack_context(&ctx.graph_nodes)
    };

    let short_term_section = ctx.short_term
        .iter()
        .map(|msg| format!("[{}] {}: {}", msg.timestamp.format("%H:%M"), msg.role, msg.content))
        .collect::<Vec<_>>()
        .join("\n");

    // Graph context xuất hiện trước (background knowledge)
    // Short-term xuất hiện sau + có timestamp → LLM tự biết đây là thông tin mới nhất
    // Không cần ghi đè thủ công — recency bias tự nhiên của LLM xử lý phần còn lại
    format!(
        "## Ký ức dài hạn (Knowledge Graph):\n{}\n\n## Hội thoại gần nhất:\n{}",
        graph_section,
        short_term_section
    )
}
```

---

## 5. Giai Đoạn Ghi: Immutable Upsert

### Bước 5.1: Phát Hiện Trái Nghĩa (Prefix + Full-Sentence Fallback)

```rust
fn is_contradiction(old_rel: &str, new_rel: &str) -> bool {
    // Rule 1: Prefix pattern — bắt cứng các cặp kinh điển, O(1)
    const NEGATION_PAIRS: &[(&str, &str)] = &[
        ("MUON_", "KHONG_MUON_"),
        ("THICH_", "KHONG_THICH_"),
        ("CO_", "KHONG_CO_"),
        ("TIN_", "KHONG_TIN_"),
    ];

    for (pos, neg) in NEGATION_PAIRS {
        if (old_rel.starts_with(pos) && new_rel.starts_with(neg))
            || (old_rel.starts_with(neg) && new_rel.starts_with(pos))
        {
            return true;
        }
    }

    // Rule 2: Full-sentence embedding fallback
    // Embed ngữ cảnh đầy đủ để model hiểu được negation
    // KHÔNG embed bare token "MUON_DI" vì word-level embedding không encode negation
    let old_ctx = format!("user {} somewhere", old_rel.to_lowercase().replace('_', " "));
    let new_ctx = format!("user {} somewhere", new_rel.to_lowercase().replace('_', " "));

    cosine_similarity(&embed(&old_ctx), &embed(&new_ctx)) < 0.3
}
```

### Bước 5.2: Không Gộp Node Vật Lý — IS_ALIAS_OF

```rust
async fn resolve_alias(
    db: &Surreal<Client>,
    node_a: &RecordId,
    node_b: &RecordId,
    session_id: &str,
) -> Result<()> {
    // Tuyệt đối không DELETE hoặc merge node
    // Chỉ tạo edge IS_ALIAS_OF — rollback bằng cách xóa edge này
    db.query("
        BEGIN TRANSACTION;
        RELATE $a -> relates_to -> $b SET
            relation_type = 'IS_ALIAS_OF',
            weight        = 1.0,
            confidence    = 1.0,
            created_at    = time::now(),
            source_session_id = $session;
        COMMIT TRANSACTION;
    ")
    .bind(("a", node_a))
    .bind(("b", node_b))
    .bind(("session", session_id))
    .await?;

    Ok(())
}
```

### Bước 5.3: ACID Upsert với Conflict Resolution (Fix race condition)

SELECT phải nằm **bên trong** Transaction. Tách SELECT ra ngoài tạo khoảng trống
giữa lần đọc và lần ghi — worker khác có thể INSERT trùng trong khoảng đó.
Toàn bộ logic được đẩy vào một câu SurrealQL duy nhất:

```rust
async fn upsert_relation(
    db: &Surreal<Client>,
    new_edge: &RelatesToEdge,
) -> Result<()> {
    // SELECT nằm trong cùng transaction với INSERT/UPDATE
    // Không có khoảng trống race condition giữa check và write
    db.query("
        BEGIN TRANSACTION;

        LET $existing = (
            SELECT * FROM relates_to
            WHERE in = $in AND out = $out
            LIMIT 1
        );

        IF $existing {
            IF is_contradiction($existing[0].relation_type, $rel_type) {
                -- Contradiction: set weight cũ về 0, insert edge mới
                UPDATE relates_to SET weight = 0.0 WHERE id = $existing[0].id;
                RELATE $in -> relates_to -> $out SET
                    relation_type     = $rel_type,
                    weight            = $weight,
                    confidence        = $confidence,
                    created_at        = time::now(),
                    source_session_id = $session;
            } ELSE {
                -- Không conflict: cập nhật weight lên max, giữ immutability
                UPDATE relates_to SET
                    weight            = math::max(weight, $weight),
                    confidence        = $confidence,
                    source_session_id = $session
                WHERE id = $existing[0].id;
            }
        } ELSE {
            -- Edge chưa tồn tại: INSERT mới
            RELATE $in -> relates_to -> $out SET
                relation_type     = $rel_type,
                weight            = $weight,
                confidence        = $confidence,
                created_at        = time::now(),
                source_session_id = $session;
        };

        COMMIT TRANSACTION;
    ")
    .bind(("in",       &new_edge.in_node))
    .bind(("out",      &new_edge.out_node))
    .bind(("rel_type", &new_edge.relation_type))
    .bind(("weight",   new_edge.weight))
    .bind(("confidence", new_edge.confidence))
    .bind(("session",  &new_edge.source_session_id))
    .await?;

    Ok(())
}
```

---

## 6. Lộ Trình Triển Khai Vào `polyverse-agent`

| Bước | File | Việc cần làm |
|------|------|-------------|
| 1 | `pa-memory/schema.rs` | Thêm `EntityNode`, `RelatesToEdge` structs |
| 2 | `pa-memory/graph_store.rs` | `fetch_subgraph()` 2 rounds, `upsert_relation()` |
| 3 | `pa-memory/activation.rs` | `calculate_activation()`, `traverse_edge()`, `build_degree_map()` |
| 4 | `pa-memory/context.rs` | `pack_context()`, `build_system_prompt()` |
| 5 | `pa-agent/social_context.rs` | Thay RAG call bằng `retrieve_cognitive_context()` |
| 6 | `pa-cognitive/workers.rs` | Đổi `SemanticCompressor` → `KnowledgeGraphWorker` |
| 7 | `pa-cognitive/extraction.rs` | LLM prompt trích xuất `[E]->[R]->[E]`, gọi `upsert_relation()` |

---

## 7. Changelog

| Vấn đề | v4.0 | v4.1 | v4.2 |
|--------|------|------|------|
| SurrealQL depth=2 | ❌ Chỉ depth=1 | ✅ 2 rounds fetch | ✅ |
| node_degree source | ❌ Undefined | ✅ Tính từ subgraph RAM | ✅ đếm đúng `in_node` (out-degree) |
| IS_ALIAS_OF traversal | ⚠️ Không define | ✅ Identity pass, weight=1 no decay | ✅ |
| Memory Shadowing | ⚠️ Vague "ghi đè" | ✅ Timestamp ordering, LLM recency bias | ✅ |
| Subgraph LIMIT | ❌ Không giới hạn | ❌ Không giới hạn | ✅ LIMIT 50/200 |
| Activation propagation | ❌ seed_score tính lại từ đầu | ❌ seed_score tính lại từ đầu | ✅ parent_activation lan truyền đúng |
| Race condition upsert | ❌ SELECT ngoài transaction | ❌ SELECT ngoài transaction | ✅ Toàn bộ trong 1 transaction |
| Token budget (tiếng Việt) | ❌ len()/4 sai với UTF-8 | ❌ len()/4 sai với UTF-8 | ✅ chars().count()/2 |