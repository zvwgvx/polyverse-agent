use surrealdb::engine::local::Mem;
use surrealdb::Surreal;
use serde_json::Value;

#[tokio::main]
async fn main() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("ns").use_db("db").await.unwrap();
    let mut resp = db.query("RETURN { a: 1 }; RETURN [1, 2];").await.unwrap();
    let v1: Option<Value> = resp.take(0).unwrap();
    println!("v1={:?}", v1);
    let v2: Vec<Value> = resp.take(1).unwrap();
    println!("v2={:?}", v2);
}
