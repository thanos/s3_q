use s3_q::{QueueAPI, QueueConfig};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Starting Simple S3 Queue Benchmark");
    
    // Create a temporary directory for the benchmark
    let temp_dir = TempDir::new()?;
    
    // Create a queue with default configuration
    let queue = QueueAPI::with_config(QueueConfig {
        max_message_size: 1024 * 1024, // 1MB
        batch_size: 100,
        db_path: temp_dir.path().to_path_buf(),
    }).await?;
    
    // Test different message sizes
    let test_sizes = vec![64, 128, 256, 512, 1024]; // 64B, 256B, 1KB, 4KB
    
    for message_size in test_sizes {
        println!("\n📊 Testing with {} byte messages", message_size);
        
        // Create payload
        let payload = vec![0u8; message_size];
        
        // Test enqueue performance
        let test_count = 1000;
        println!("   - Enqueueing {} messages...", test_count);
        
        let enqueue_start = Instant::now();
        for i in 0..test_count {
            queue.enqueue(payload.clone()).await?;
            if (i + 1) % 100 == 0 {
                print!(".");
            }
        }
        let enqueue_duration = enqueue_start.elapsed();
        let enqueue_rate = test_count as f64 / enqueue_duration.as_secs_f64();
        
        println!("\n   - Enqueue: {:.0} msg/s ({:.2?} total)", enqueue_rate, enqueue_duration);
        
        // Test dequeue performance
        println!("   - Dequeueing {} messages...", test_count);
        
        let dequeue_start = Instant::now();
        let mut dequeued_count = 0;
        
        while dequeued_count < test_count {
            if let Some(message) = queue.dequeue().await? {
                // Verify message size
                if message.payload.len() != message_size {
                    println!("   - WARNING: Expected {} bytes, got {} bytes", message_size, message.payload.len());
                }
                
                queue.complete(message.id).await?;
                dequeued_count += 1;
                
                if dequeued_count % 100 == 0 {
                    print!(".");
                }
            } else {
                break;
            }
        }
        
        let dequeue_duration = dequeue_start.elapsed();
        let dequeue_rate = test_count as f64 / dequeue_duration.as_secs_f64();
        
        println!("\n   - Dequeue: {:.0} msg/s ({:.2?} total)", dequeue_rate, dequeue_duration);
        
        // Cleanup
        let cleanup_start = Instant::now();
        let cleaned = queue.cleanup().await?;
        let cleanup_duration = cleanup_start.elapsed();
        
        println!("   - Cleanup: {} messages in {:.2?}", cleaned, cleanup_duration);
        
        // Get stats
        let stats = queue.get_stats().await?;
        println!("   - Final stats: {:?}", stats);
    }
    
    // Close the queue
    queue.close().await?;
    
    println!("\n🎉 Simple benchmark completed!");
    Ok(())
} 