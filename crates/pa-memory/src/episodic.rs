use anyhow::{Context, Result};
use arrow::array::{
    ArrayRef, Float32Array, Int64Array, RecordBatch, StringArray,
};
use arrow::datatypes::{DataType, Field, Schema};
use lancedb::{query::{ExecutableQuery, QueryBase}, Table};
use std::sync::Arc;
use futures::StreamExt;

/// Represents a single chunk of memory in the episodic store.
#[derive(Debug, Clone)]
pub struct MemoryEvent {
    pub id: String,
    pub vector: Vec<f32>,
    pub content: String,
    pub timestamp: i64,
    pub importance: f32,
    pub metadata: String,
}

pub struct EpisodicStore {
    table: Table,
}

impl EpisodicStore {
    /// Opens the LanceDB database and ensures the table exists.
    pub async fn open(uri: &str, table_name: &str) -> Result<Self> {
        let conn = lancedb::connect(uri).execute().await
            .context("Failed to connect to LanceDB")?;

        let table_names = conn.table_names().execute().await?;
        let table = if table_names.contains(&table_name.to_string()) {
            conn.open_table(table_name).execute().await?
        } else {
            let schema = Self::schema();
            conn.create_empty_table(table_name, schema)
                .execute()
                .await?
        };

        Ok(Self { table })
    }

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 384),
                false,
            ),
            Field::new("content", DataType::Utf8, false),
            Field::new("timestamp", DataType::Int64, false),
            Field::new("importance", DataType::Float32, false),
            Field::new("metadata", DataType::Utf8, false),
        ]))
    }

    pub async fn insert(&self, events: Vec<MemoryEvent>) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let schema = Self::schema();

        // Build arrays
        let id_array = StringArray::from(events.iter().map(|e| e.id.clone()).collect::<Vec<_>>());
        
        let mut vector_builder = arrow::array::FixedSizeListBuilder::new(
            arrow::array::Float32Builder::new(),
            384,
        );
        for event in &events {
            vector_builder.values().append_slice(&event.vector);
            vector_builder.append(true);
        }
        let vector_array = vector_builder.finish();

        let content_array = StringArray::from(events.iter().map(|e| e.content.clone()).collect::<Vec<_>>());
        let timestamp_array = Int64Array::from(events.iter().map(|e| e.timestamp).collect::<Vec<_>>());
        let importance_array = Float32Array::from(events.iter().map(|e| e.importance).collect::<Vec<_>>());
        let metadata_array = StringArray::from(events.iter().map(|e| e.metadata.clone()).collect::<Vec<_>>());

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(id_array) as ArrayRef,
                Arc::new(vector_array) as ArrayRef,
                Arc::new(content_array) as ArrayRef,
                Arc::new(timestamp_array) as ArrayRef,
                Arc::new(importance_array) as ArrayRef,
                Arc::new(metadata_array) as ArrayRef,
            ],
        ).context("Failed to build RecordBatch")?;

        self.table.add(Box::new(arrow::array::RecordBatchIterator::new(
            vec![Ok(batch)],
            schema,
        )))
        .execute()
        .await
        .context("Failed to insert records into LanceDB")?;

        Ok(())
    }

    /// Search the memory store with a query vector, applying time-weighted re-ranking.
    /// Formula: Final_Score = Cosine_Similarity * e^(-λ * Δt_days) * Importance_Weight
    pub async fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
        lambda: f32, // e.g. 0.05
    ) -> Result<Vec<MemoryEvent>> {
        let mut stream = self.table
            .query()
            .nearest_to(query_vector)?
            .limit(20)
            .execute()
            .await?;
        
        let mut candidates = Vec::new();

        while let Some(batch_res) = stream.next().await {
            let batch = batch_res?;
            
            // Extract arrays
            let id_col = batch.column_by_name("id").context("Missing id")?
                .as_any().downcast_ref::<StringArray>().context("id not a string")?;
            let vector_col = batch.column_by_name("vector").context("Missing vector")?
                .as_any().downcast_ref::<arrow::array::FixedSizeListArray>().context("vector not a list")?;
            let content_col = batch.column_by_name("content").context("Missing content")?
                .as_any().downcast_ref::<StringArray>().context("content not a string")?;
            let timestamp_col = batch.column_by_name("timestamp").context("Missing timestamp")?
                .as_any().downcast_ref::<Int64Array>().context("timestamp not i64")?;
            let importance_col = batch.column_by_name("importance").context("Missing importance")?
                .as_any().downcast_ref::<Float32Array>().context("importance not f32")?;
            let metadata_col = batch.column_by_name("metadata").context("Missing metadata")?
                .as_any().downcast_ref::<StringArray>().context("metadata not string")?;
            let distance_col = batch.column_by_name("_distance").context("Missing _distance")?
                .as_any().downcast_ref::<Float32Array>().context("_distance not f32")?;

            let float_values = vector_col.values().as_any().downcast_ref::<Float32Array>().unwrap();

            for i in 0..batch.num_rows() {
                let id = id_col.value(i).to_string();
                let content = content_col.value(i).to_string();
                let timestamp = timestamp_col.value(i);
                let importance = importance_col.value(i);
                let metadata = metadata_col.value(i).to_string();
                let distance = distance_col.value(i);

                let start_idx = vector_col.value_offset(i) as usize;
                let end_idx = start_idx + vector_col.value_length() as usize;
                let vector = float_values.values()[start_idx..end_idx].to_vec();

                candidates.push((MemoryEvent { id, vector, content, timestamp, importance, metadata }, distance));
            }
        }

        let now = chrono::Utc::now().timestamp();
        
        // Re-ranking based on time and importance
        candidates.sort_by(|(a, dist_a), (b, dist_b)| {
            let score_a = calculate_final_score(*dist_a, a.timestamp, a.importance, now, lambda);
            let score_b = calculate_final_score(*dist_b, b.timestamp, b.importance, now, lambda);
            // Sort descending: highest score first
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Take top K
        Ok(candidates.into_iter().take(limit).map(|(event, _)| event).collect())
    }

    /// Count total memory chunks stored for a specific user
    pub async fn count_user_chunks(&self, username: &str) -> Result<usize> {
        let filter_expr = format!("metadata LIKE '%\"username\":\"{}\"%'", username);
        let mut stream = self.table
            .query()
            .only_if(filter_expr)
            .execute()
            .await?;
            
        let mut count = 0;
        while let Some(batch_res) = stream.next().await {
            if let Ok(batch) = batch_res {
                count += batch.num_rows();
            }
        }
        
        Ok(count)
    }
}

fn calculate_final_score(distance: f32, timestamp: i64, importance: f32, now: i64, lambda: f32) -> f32 {
    // LanceDB usually returns L2 distance. Cosine similarity is roughly 1 - (L2^2) / 2 if normalized.
    // Let similarity = 1.0 / (1.0 + distance)
    let similarity = 1.0 / (1.0 + distance);
    let dt_seconds = (now - timestamp).max(0);
    let dt_days = dt_seconds as f32 / 86400.0;
    
    let time_decay = (-lambda * dt_days).exp();
    similarity * time_decay * importance
}
