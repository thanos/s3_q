use slatedb::Db;
use std::time::Instant;
use tempfile::TempDir;
use std::sync::Arc;
use object_store::local::LocalFileSystem;
use slatedb::object_store::{ObjectStore, memory::InMemory};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of operations to perform
    #[arg(short, long, default_value_t = 10000)]
    count: usize,
    
    /// Size of each value in bytes
    #[arg(short, long, default_value_t = 1024)]
    value_size: usize,
    
    /// Enable batch operations
    #[arg(short, long)]
    batch: bool,
    
    /// Batch size for batch operations
    #[arg(long, default_value_t = 100)]
    batch_size: usize,
    
    /// Test get operations after writes
    #[arg(short, long)]
    test_get: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    
    println!("🚀 Starting SlateDB Benchmark");
    println!("📊 Configuration:");
    println!("   - Operations: {}", args.count);
    println!("   - Value size: {} bytes", args.value_size);
    println!("   - Batch mode: {}", args.batch);
    if args.batch {
        println!("   - Batch size: {}", args.batch_size);
    }
    println!("   - Test get operations: {}", args.test_get);
    
    // Create a temporary directory for the benchmark
    let temp_dir = TempDir::new()?;
    
    // Create a file-based object store for SlateDB persistence
    let object_store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());

    let db = Db::open(temp_dir.path().to_string_lossy().as_ref(), object_store).await?;
    
    // Create test data
    let value = vec![0u8; args.value_size];
    
    if args.batch {
        println!("\n📈 Testing Batch Write Performance...");
        benchmark_batch_writes(&db, &value, args.count, args.batch_size).await?;
    } else {
        println!("\n📈 Testing Individual Put Performance...");
        benchmark_individual_puts(&db, &value, args.count).await?;
    }
    
    if args.test_get {
        println!("\n🔍 Testing Get Performance...");
        benchmark_get_operations(&db, args.count).await?;
    }
    
    // Close the database
    db.close().await?;
    
    println!("\n🎉 Database benchmark completed!");
    Ok(())
}

async fn benchmark_individual_puts(db: &Db, value: &[u8], count: usize) -> anyhow::Result<()> {
    let start = Instant::now();
    
    for i in 0..count {
        let key = format!("key_{:08}", i).into_bytes();
        db.put(&key, value).await?;
        
        if (i + 1) % 1000 == 0 {
            let elapsed = start.elapsed();
            let rate = (i + 1) as f64 / elapsed.as_secs_f64();
            println!("   - Progress: {}/{} ({:.0} ops/s)", i + 1, count, rate);
        }
    }
    
    let duration = start.elapsed();
    let rate = count as f64 / duration.as_secs_f64();
    
    println!("✅ Individual Put Complete!");
    println!("   - Total time: {:.2?}", duration);
    println!("   - Rate: {:.0} operations/second", rate);
    println!("   - Average time per operation: {:.2?}", duration / count as u32);
    
    Ok(())
}

async fn benchmark_batch_writes(db: &Db, value: &[u8], total_count: usize, batch_size: usize) -> anyhow::Result<()> {
    let start = Instant::now();
    let mut batch = Vec::new();
    let mut total_operations = 0;
    
    for i in 0..total_count {
        let key = format!("key_{:08}", i).into_bytes();
        batch.push((key, value.to_vec()));
        
        if batch.len() >= batch_size || i == total_count - 1 {
            // Write batch
            for (key, val) in &batch {
                db.put(key, val).await?;
                total_operations += 1;
            }
            
            batch.clear();
            
            if total_operations % 1000 == 0 {
                let elapsed = start.elapsed();
                let rate = total_operations as f64 / elapsed.as_secs_f64();
                println!("   - Progress: {}/{} ({:.0} ops/s)", total_operations, total_count, rate);
            }
        }
    }
    
    let duration = start.elapsed();
    let rate = total_operations as f64 / duration.as_secs_f64();
    
    println!("✅ Batch Write Complete!");
    println!("   - Total time: {:.2?}", duration);
    println!("   - Rate: {:.0} operations/second", rate);
    println!("   - Average time per operation: {:.2?}", duration / total_operations as u32);
    println!("   - Batch size: {}", batch_size);
    
    Ok(())
}

async fn benchmark_get_operations(db: &Db, count: usize) -> anyhow::Result<()> {
    println!("   - Testing get operations...");
    
    let start = Instant::now();
    let mut get_count = 0;
    
    // Test random get operations
    for i in 0..count {
        let key = format!("key_{:08}", i).into_bytes();
        if let Ok(Some(_value)) = db.get(&key).await {
            get_count += 1;
        }
        
        if (i + 1) % 1000 == 0 {
            let elapsed = start.elapsed();
            let rate = (i + 1) as f64 / elapsed.as_secs_f64();
            println!("     - Progress: {}/{} ({:.0} ops/s)", i + 1, count, rate);
        }
    }
    
    let duration = start.elapsed();
    let rate = get_count as f64 / duration.as_secs_f64();
    
    println!("✅ Get Operations Complete!");
    println!("   - Total time: {:.2?}", duration);
    println!("   - Successful gets: {}", get_count);
    println!("   - Rate: {:.0} operations/second", rate);
    println!("   - Expected: {}", count);
    
    Ok(())
} 