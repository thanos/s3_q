use slatedb::{WriteBatch, Db, config::WriteOptions, SlateDBError};
use slatedb::object_store::{ObjectStore, memory::InMemory};
use std::sync::Arc;
use std::time::{Duration, Instant};
use object_store::aws::S3ConditionalPut;

#[tokio::main]
async fn main() -> Result<(), SlateDBError> {
    // let object_store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let object_store = Arc::new(
        object_store::aws::AmazonS3Builder::new()
            // These will be different if you are using real AWS
            .with_allow_http(true)
            .with_endpoint("http://localhost:4566")
            .with_access_key_id("test")
            .with_secret_access_key("test")
            .with_bucket_name("slatedb")
            .with_region("us-east-1")
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            .build()?,
    );
    let db = Db::open("test_db", object_store).await?;

    let value = vec![0u8; 4096];
    let start = Instant::now();
    let count = 1000;
    for i in 0..count {
        let key = format!("key_{:08}", i).into_bytes();
        db.put(key, value.to_vec()).await?;
    }
    db.close().await?;
    let duration = start.elapsed();
    println!("Time taken: {:?}", duration);
    let elapsed = start.elapsed();
    let rate = count as f64 / elapsed.as_secs_f64();
    println!("   - single write rate: ({:.0} ops/s)",rate);
    Ok(())
}