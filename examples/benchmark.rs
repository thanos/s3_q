use s3_q::{QueueAPI, QueueConfig};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use std::sync::Arc;
use tokio::sync::Semaphore;
use std::env;

/// Helper function to format numbers with commas for readability
fn format_number(n: usize) -> String {
    n.to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap())
        .collect::<Vec<_>>()
        .join(",")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Get number of messages from command line argument, default to 10,000 for testing
    let total_messages = env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10_000);
    
    println!("🚀 Starting S3 Queue Benchmark");
    println!("📊 Testing with {} messages of 1KB each", format_number(total_messages));
    
    // Create a temporary directory for the benchmark
    let temp_dir = TempDir::new()?;
    
    // Create a queue with optimized configuration for benchmarking
    let queue = QueueAPI::with_config(QueueConfig {
        max_message_size: 1024 * 1024, // 1MB
        batch_size: 1000, // Larger batch size for better performance
        db_path: temp_dir.path().to_path_buf(),
    }).await?;
    
    let queue = Arc::new(queue);
    
    // Benchmark parameters
    let message_size = 1024; // 1KB
    let concurrent_workers = std::cmp::min(16, total_messages); // Number of concurrent enqueue workers
    
    // Create a 1KB payload
    let payload = vec![0u8; message_size];
    
    println!("\n📈 Starting Enqueue Benchmark...");
    println!("   - Total messages: {}", format_number(total_messages));
    println!("   - Message size: {} bytes", message_size);
    println!("   - Concurrent workers: {}", concurrent_workers);
    
    // Enqueue benchmark
    let enqueue_start = Instant::now();
    let enqueue_semaphore = Arc::new(Semaphore::new(concurrent_workers));
    let mut enqueue_handles = Vec::new();
    
    let step_size = std::cmp::max(1, total_messages / concurrent_workers);
    for batch_start in (0..total_messages).step_by(step_size) {
        let batch_end = (batch_start + step_size).min(total_messages);
        let queue_clone = Arc::clone(&queue);
        let payload_clone = payload.clone();
        let semaphore_clone = Arc::clone(&enqueue_semaphore);
        
        let handle = tokio::spawn(async move {
            let _permit = semaphore_clone.acquire().await.unwrap();
            
            for _ in batch_start..batch_end {
                queue_clone.enqueue(payload_clone.clone()).await.unwrap();
            }
        });
        
        enqueue_handles.push(handle);
    }
    
    // Wait for all enqueue operations to complete
    for handle in enqueue_handles {
        handle.await.unwrap();
    }
    
    let enqueue_duration = enqueue_start.elapsed();
    let enqueue_rate = total_messages as f64 / enqueue_duration.as_secs_f64();
    
    println!("\n✅ Enqueue Benchmark Complete!");
    println!("   - Total time: {:.2?}", enqueue_duration);
    println!("   - Rate: {:.0} messages/second", enqueue_rate);
    println!("   - Average time per message: {:.2?}", enqueue_duration / total_messages as u32);
    
    // Get stats after enqueueing
    let stats_after_enqueue = queue.get_stats().await?;
    println!("   - Queue stats: {:?}", stats_after_enqueue);
    
    // Wait a moment for any background processing
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    println!("\n📉 Starting Dequeue Benchmark...");
    
    // Dequeue benchmark
    let dequeue_start = Instant::now();
    let mut dequeued_count = 0;
    let mut total_dequeue_time = Duration::ZERO;
    let mut dequeue_times = Vec::new();
    
    while dequeued_count < total_messages {
        let dequeue_start_single = Instant::now();
        
        if let Some(message) = queue.dequeue().await? {
            let dequeue_duration_single = dequeue_start_single.elapsed();
            dequeue_times.push(dequeue_duration_single);
            total_dequeue_time += dequeue_duration_single;
            
            // Mark as completed immediately
            queue.complete(message.id).await?;
            dequeued_count += 1;
            
            // Progress indicator every 10,000 messages (or every 1,000 for smaller tests)
            let progress_interval = if total_messages >= 100_000 { 100_000 } else { 1_000 };
            if dequeued_count % progress_interval == 0 {
                let elapsed = dequeue_start.elapsed();
                let rate = dequeued_count as f64 / elapsed.as_secs_f64();
                println!("   - Dequeued: {} messages, Rate: {:.0} msg/s", format_number(dequeued_count), rate);
            }
        } else {
            // No more messages available
            break;
        }
    }
    
    let dequeue_duration = dequeue_start.elapsed();
    let dequeue_rate = total_messages as f64 / dequeue_duration.as_secs_f64();
    
    println!("\n✅ Dequeue Benchmark Complete!");
    println!("   - Total time: {:.2?}", dequeue_duration);
    println!("   - Rate: {:.0} messages/second", dequeue_rate);
    println!("   - Average time per message: {:.2?}", dequeue_duration / total_messages as u32);
    
    // Calculate dequeue timing statistics
    if !dequeue_times.is_empty() {
        dequeue_times.sort();
        let min_dequeue = dequeue_times.first().unwrap();
        let max_dequeue = dequeue_times.last().unwrap();
        let median_dequeue = dequeue_times[dequeue_times.len() / 2];
        let avg_dequeue = total_dequeue_time / total_messages as u32;
        
        println!("   - Dequeue timing statistics:");
        println!("     - Min: {:.2?}", min_dequeue);
        println!("     - Max: {:.2?}", max_dequeue);
        println!("     - Median: {:.2?}", median_dequeue);
        println!("     - Average: {:.2?}", avg_dequeue);
    }
    
    // Get final stats
    let final_stats = queue.get_stats().await?;
    println!("   - Final queue stats: {:?}", final_stats);
    
    // Cleanup
    println!("\n🧹 Cleaning up...");
    let cleanup_start = Instant::now();
    let cleaned = queue.cleanup().await?;
    let cleanup_duration = cleanup_start.elapsed();
    
    println!("   - Cleaned up {} messages in {:.2?}", format_number(cleaned), cleanup_duration);
    
    // Final stats after cleanup
    let stats_after_cleanup = queue.get_stats().await?;
    println!("   - Stats after cleanup: {:?}", stats_after_cleanup);
    
    // Close the queue
    queue.close().await?;
    
    println!("\n📊 Benchmark Summary:");
    println!("   - Enqueue: {:.0} msg/s ({:.2?} total)", enqueue_rate, enqueue_duration);
    println!("   - Dequeue: {:.0} msg/s ({:.2?} total)", dequeue_rate, dequeue_duration);
    println!("   - Total throughput: {:.0} operations/second", 
             (total_messages as f64 * 2.0) / (enqueue_duration + dequeue_duration).as_secs_f64());
    
    println!("\n🎉 Benchmark completed successfully!");
    println!("💡 Tip: Run with a number argument to test different message counts (e.g., 'cargo run --example benchmark 100000')");
    Ok(())
} 