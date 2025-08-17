# S3 Queue (s3_q)

A high-performance queuing system built with Rust, featuring S3-compatible storage and a clean API interface.

## Features

- **Priority-based queuing**: Messages are processed in order of priority
- **Retry mechanism**: Automatic retry with configurable limits
- **Async/await support**: Built with Tokio for high-performance async operations
- **S3-compatible storage**: Uses SlateDB with S3-compatible object storage
- **Thread-safe**: Built with Arc and RwLock for concurrent access
- **Configurable**: Customizable retry limits, message sizes, and batch processing

## Quick Start

### Basic Usage

```rust
use s3_q::{QueueAPI, QueueConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a new queue with default configuration
    let queue = QueueAPI::new();
    
    // Enqueue a message
    let message_id = queue.enqueue(b"Hello World".to_vec(), 1).await?;
    
    // Dequeue and process
    if let Some(message) = queue.dequeue().await? {
        println!("Processing: {}", String::from_utf8_lossy(&message.payload));
        
        // Mark as completed
        queue.complete(&message.id).await?;
    }
    
    Ok(())
}
```

### Custom Configuration

```rust
use s3_q::{QueueAPI, QueueConfig};

let config = QueueConfig {
    max_retries: 5,
    retry_delay_ms: 2000,
    max_message_size: 128 * 1024 * 1024, // 128MB
    batch_size: 50,
};

let queue = QueueAPI::with_config(config);
```

## API Reference

### Core Types

- **`Message`**: Represents a message in the queue with ID, payload, timestamp, priority, and retry count
- **`MessageStatus`**: Tracks message state (Pending, Processing, Completed, Failed, Retry)
- **`QueueConfig`**: Configuration options for the queue behavior
- **`QueueStats`**: Statistics about queue performance and message counts

### Main Methods

- **`enqueue(payload, priority)`**: Add a new message to the queue
- **`dequeue()`**: Get the next highest-priority pending message
- **`complete(message_id)`**: Mark a message as successfully processed
- **`fail(message_id)`**: Mark a message as failed (triggers retry if within limits)
- **`get_status(message_id)`**: Check the current status of a message
- **`get_stats()`**: Get comprehensive queue statistics
- **`cleanup()`**: Remove completed and failed messages

## Examples

Run the included example to see the API in action:

```bash
cargo run --example queue_example
```

## Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `max_retries` | 3 | Maximum number of retry attempts for failed messages |
| `retry_delay_ms` | 1000 | Delay between retry attempts in milliseconds |
| `max_message_size` | 64MB | Maximum allowed size for individual messages |
| `batch_size` | 100 | Batch size for bulk operations |

## Message Priority

Messages are processed in priority order (higher numbers = higher priority). Priority values range from 0-255, where:
- 0: Lowest priority
- 255: Highest priority

## Error Handling

The API uses `anyhow::Result` for error handling and includes custom error types for common failure scenarios:

- `MessageNotFound`: Attempted to access a non-existent message
- `MessageTooLarge`: Message exceeds configured size limits
- `QueueFull`: Queue has reached capacity
- `InvalidPriority`: Priority value outside valid range

## Thread Safety

The `QueueAPI` is designed for concurrent access and can be safely shared across multiple threads using `Arc<QueueAPI>`. All operations are protected by async read-write locks for optimal performance.

## Performance Considerations

- Messages are stored in memory for fast access
- Priority-based dequeuing ensures high-priority messages are processed first
- Async operations prevent blocking during I/O operations
- Configurable batch sizes allow tuning for your specific workload

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
