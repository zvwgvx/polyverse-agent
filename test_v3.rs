use surrealdb::engine::local::Mem;
use surrealdb::Surreal;

#[tokio::main]
async fn main() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("ns").use_db("db").await.unwrap();
    let res = db.query("RETURN type::record('person', 'ryuuko');").await;
    println!("{:?}", res);
}
