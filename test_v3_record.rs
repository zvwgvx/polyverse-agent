use surrealdb::engine::local::Mem;
use surrealdb::{Surreal, RecordId};
use serde_json::Value;

#[tokio::main]
async fn main() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("ns").use_db("db").await.unwrap();
    let user_id = RecordId::from(("person", "tester"));
    let mut resp = db.query("CREATE person:ryuuko; CREATE type::table($tb); RELATE person:ryuuko->feels->$user_id;")
        .bind(("tb", "tester"))
        .bind(("user_id", user_id))
        .await;
    println!("{:?}", resp);
}
