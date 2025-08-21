mod api;

use object_store::aws::S3ConditionalPut;
use slatedb::Db;
use std::sync::Arc;
use api::{QueueAPI, QueueConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize the queue API with a custom database path
    let queue_config = QueueConfig {
        max_message_size: 128 * 1024 * 1024, // 128MB
        batch_size: 50,
        db_path: PathBuf::from("/tmp/s3_q_demo"),
    };
    
    let queue_api = QueueAPI::with_config(queue_config).await?;
    
    // Example: Enqueue some messages
    let message1 = b"Hello from S3 Queue with SlateDB!".to_vec();
    let message2 = b"Another message".to_vec();
    
    let msg1_id = queue_api.enqueue(message1).await?;
    let msg2_id = queue_api.enqueue(message2).await?;
    
    println!("Enqueued messages: {} and {}", msg1_id, msg2_id);
    
    // Example: Get queue statistics
    let stats = queue_api.get_stats().await?;
    println!("Queue stats: {:?}", stats);
    
    // Example: Process messages
    if let Some(message) = queue_api.dequeue().await? {
        println!("Processing message: {} (ID: {})", message.id, message.id);
        // Simulate processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        // Mark as completed
        queue_api.complete(message.id).await?;
        println!("Message {} completed", message.id);
    }
    
    // Get updated stats
    let updated_stats = queue_api.get_stats().await?;
    println!("Updated stats: {:?}", updated_stats);

    // Example: Demonstrate persistence by closing and reopening
    println!("\nDemonstrating persistence...");
    queue_api.close().await?;
    
    // Reopen the same queue
    let queue_config2 = QueueConfig {
        db_path: PathBuf::from("/tmp/s3_q_demo"),
        ..Default::default()
    };
    let queue_api2 = QueueAPI::with_config(queue_config2).await?;
    
    // Check that messages still exist
    let msg1 = queue_api2.get_message(msg1_id).await?;
    let msg2 = queue_api2.get_message(msg2_id).await?;
    
    if let Some(message) = msg1 {
        println!("Retrieved persisted message: {} (ID: {})", message.id, message.id);
    }
    
    if let Some(message) = msg2 {
        println!("Retrieved persisted message: {} (ID: {})", message.id, message.id);
    }
    
    // Cleanup
    queue_api2.close().await?;

    // Original S3 and SlateDB code
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

    let db = Db::open("/tmp/slatedb_s3_compatible", object_store.clone()).await?;

    // Call db.put with a key and a 64 meg value to trigger L0 SST flush
    let value: Vec<u8> = vec![0; 64 * 1024 * 1024];
    db.put(b"k1", value.as_slice()).await?;
    db.close().await?;

    Ok(())
}
