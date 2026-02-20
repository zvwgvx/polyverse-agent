use lancedb::{connect, Table};
use arrow::array::{RecordBatch, StringArray, Float32Array, Int64Array};
use arrow::datatypes::{Schema, Field, DataType};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    println!("hello");
}
