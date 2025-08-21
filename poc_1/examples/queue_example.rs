use s3_q::{QueueAPI, QueueConfig};
use tempfile::TempDir;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Starting S3 Queue with SlateDB Example");
    
    // Create a temporary directory for the demo
    let temp_dir = TempDir::new()?;
    
    // Create a queue with custom configuration
    let queue = QueueAPI::with_config(QueueConfig {
        max_message_size: 1024 * 1024, // 1MB
        batch_size: 10,
        db_path: temp_dir.path().to_path_buf(),
    }).await?;
    
    // Enqueue several messages
    let messages = vec![
        "First task",
        "Second task",
        "Third task",
        "Fourth task",
    ];
    
    let mut message_ids = Vec::new();
    
    for content in messages {
        let payload = content.as_bytes().to_vec();
        let id = queue.enqueue(payload).await?;
        message_ids.push((id, content));
        println!("✅ Enqueued: {} (ID: {})", content, id);
    }
    
    // Show initial queue stats
    let stats = queue.get_stats().await?;
    println!("\n📊 Initial Queue Stats: {:?}", stats);
    
    // Process messages in FIFO order
    println!("\n🔄 Processing messages...");
    
    for _ in 0..message_ids.len() {
        if let Some(message) = queue.dequeue().await? {
            println!("📨 Processing: {} (ID: {})", 
                String::from_utf8_lossy(&message.payload), 
                message.id
            );
            
            // Simulate some processing time
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            
            // Randomly fail some messages to test failure handling
            if rand::random::<f32>() < 0.3 {
                println!("❌ Message {} failed", message.id);
                queue.fail(message.id).await?;
            } else {
                println!("✅ Message {} completed successfully", message.id);
                queue.complete(message.id).await?;
            }
        }
    }
    
    // Show final stats
    let final_stats = queue.get_stats().await?;
    println!("\n📊 Final Queue Stats: {:?}", final_stats);
    
    // Cleanup completed and failed messages
    let cleaned = queue.cleanup().await?;
    println!("\n🧹 Cleaned up {} messages", cleaned);
    
    // Show stats after cleanup
    let clean_stats = queue.get_stats().await?;
    println!("📊 Stats after cleanup: {:?}", clean_stats);
    
    // Demonstrate persistence by closing and reopening
    println!("\n💾 Demonstrating persistence...");
    queue.close().await?;
    
    // Reopen the same queue
    let queue2 = QueueAPI::with_config(QueueConfig {
        db_path: temp_dir.path().to_path_buf(),
        ..Default::default()
    }).await?;
    
    // Check that some messages still exist (pending ones)
    for (id, content) in &message_ids {
        if let Some(message) = queue2.get_message(*id).await? {
            println!("📋 Retrieved persisted message: {} (ID: {})", content, message.id);
        }
    }
    
    // Cleanup
    queue2.close().await?;
    
    println!("\n🎉 Example completed successfully!");
    Ok(())
} 