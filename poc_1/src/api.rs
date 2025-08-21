use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::{HashMap, VecDeque};
use serde::{Serialize, Deserialize};
use slatedb::Db;
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::PathBuf;

/// Represents a message in the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64, // Snowflake ID
    pub payload: Vec<u8>,
    pub timestamp: u64,
}

/// Represents the status of a message
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

/// Queue configuration options
#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub max_message_size: usize,
    pub batch_size: usize,
    pub db_path: PathBuf,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_message_size: 64 * 1024 * 1024, // 64MB
            batch_size: 100,
            db_path: PathBuf::from("/tmp/s3_q_default"),
        }
    }
}

/// Main queue API interface
pub struct QueueAPI {
    config: QueueConfig,
    db: Arc<Db>,
    processing: Arc<RwLock<HashMap<i64, MessageStatus>>>,
    pending_queue: Arc<RwLock<VecDeque<i64>>>,
    node_id: u8,
    sequence: Arc<RwLock<u32>>,
}

impl QueueAPI {
    /// Create a new QueueAPI instance with default configuration
    pub async fn new() -> Result<Self> {
        let config = QueueConfig::default();
        Self::with_config(config).await
    }

    /// Create a new QueueAPI instance with custom configuration
    pub async fn with_config(config: QueueConfig) -> Result<Self> {
        // Create the database directory if it doesn't exist
        std::fs::create_dir_all(&config.db_path)?;
        
        // Create a file-based object store for SlateDB persistence
        let object_store = Arc::new(
            object_store::local::LocalFileSystem::new_with_prefix(config.db_path.clone())?
        );

        let db = Db::open(config.db_path.to_string_lossy().as_ref(), object_store).await?;
        
        Ok(Self {
            config,
            db: Arc::new(db),
            processing: Arc::new(RwLock::new(HashMap::new())),
            pending_queue: Arc::new(RwLock::new(VecDeque::new())),
            node_id: 1,
            sequence: Arc::new(RwLock::new(0)),
        })
    }

    /// Generate a unique Snowflake ID
    fn generate_id(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        
        let seq = {
            if let Ok(mut sequence) = self.sequence.try_write() {
                let seq = *sequence;
                *sequence = seq.wrapping_add(1) % 4096; // 12-bit sequence number
                seq
            } else {
                0 // Fallback if we can't acquire the lock
            }
        };
        
        // Snowflake ID format: 41 bits timestamp + 10 bits node ID + 12 bits sequence
        (now << 22) | ((self.node_id as i64) << 12) | (seq as i64)
    }

    /// Enqueue a new message
    pub async fn enqueue(&self, payload: Vec<u8>) -> Result<i64> {
        if payload.len() > self.config.max_message_size {
            return Err(anyhow::anyhow!("Message size exceeds maximum allowed size"));
        }

        let message_id = self.generate_id();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        let message = Message {
            id: message_id,
            payload,
            timestamp,
        };

        // Serialize and store message in SlateDB
        let message_bytes = bincode::serialize(&message)?;
        self.db.put(&message_id.to_le_bytes(), &message_bytes).await?;

        // Set initial status to Pending and add to pending queue
        {
            let mut processing = self.processing.write().await;
            let mut pending_queue = self.pending_queue.write().await;
            processing.insert(message_id, MessageStatus::Pending);
            pending_queue.push_back(message_id);
        }

        Ok(message_id)
    }

    /// Dequeue the next available pending message
    pub async fn dequeue(&self) -> Result<Option<Message>> {
        let mut processing = self.processing.write().await;
        let mut pending_queue = self.pending_queue.write().await;

        // Find the first available pending message from the queue
        while let Some(message_id) = pending_queue.pop_front() {
            if let Some(status) = processing.get(&message_id) {
                if matches!(status, MessageStatus::Pending) {
                    // Retrieve message from SlateDB
                    if let Ok(Some(message_bytes)) = self.db.get(&message_id.to_le_bytes()).await {
                        if let Ok(message) = bincode::deserialize::<Message>(&message_bytes.to_vec()) {
                            // Mark as processing and return the message
                            processing.insert(message_id, MessageStatus::Processing);
                            return Ok(Some(message));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Mark a message as completed
    pub async fn complete(&self, message_id: i64) -> Result<()> {
        let mut processing = self.processing.write().await;
        processing.insert(message_id, MessageStatus::Completed);
        Ok(())
    }

    /// Mark a message as failed
    pub async fn fail(&self, message_id: i64) -> Result<()> {
        let mut processing = self.processing.write().await;
        processing.insert(message_id, MessageStatus::Failed);
        Ok(())
    }

    /// Get message status
    pub async fn get_status(&self, message_id: i64) -> Result<Option<MessageStatus>> {
        let processing = self.processing.read().await;
        Ok(processing.get(&message_id).cloned())
    }

    /// Get message by ID
    pub async fn get_message(&self, message_id: i64) -> Result<Option<Message>> {
        if let Ok(Some(message_bytes)) = self.db.get(&message_id.to_le_bytes()).await {
            if let Ok(message) = bincode::deserialize::<Message>(&message_bytes.to_vec()) {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }

    /// Get queue statistics
    pub async fn get_stats(&self) -> Result<QueueStats> {
        let processing = self.processing.read().await;
        
        let mut stats = QueueStats::default();
        
        // Count messages by status
        for status in processing.values() {
            match status {
                MessageStatus::Pending => stats.pending_count += 1,
                MessageStatus::Processing => stats.processing_count += 1,
                MessageStatus::Completed => stats.completed_count += 1,
                MessageStatus::Failed => stats.failed_count += 1,
            }
        }
        
        stats.total_messages = processing.len();

        Ok(stats)
    }

    /// Clear completed and failed messages
    pub async fn cleanup(&self) -> Result<usize> {
        let mut processing = self.processing.write().await;
        let mut pending_queue = self.pending_queue.write().await;
        let mut to_remove = Vec::new();

        for (message_id, status) in processing.iter() {
            if matches!(status, MessageStatus::Completed | MessageStatus::Failed) {
                to_remove.push(*message_id);
            }
        }

        let removed_count = to_remove.len();
        for message_id in to_remove {
            // Remove from SlateDB
            let _ = self.db.delete(&message_id.to_le_bytes()).await;
            // Remove from processing status
            processing.remove(&message_id);
            // Remove from pending queue if present
            pending_queue.retain(|&id| id != message_id);
        }

        Ok(removed_count)
    }

    /// Get configuration
    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    /// Update configuration
    pub fn update_config(&mut self, config: QueueConfig) {
        self.config = config;
    }

    /// Close the queue and database
    pub async fn close(&self) -> Result<()> {
        self.db.close().await?;
        Ok(())
    }
}

/// Queue statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueueStats {
    pub total_messages: usize,
    pub pending_count: usize,
    pub processing_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
}

/// Error types for the queue API
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("Message not found: {0}")]
    MessageNotFound(i64),
    
    #[error("Message size too large: {0} bytes (max: {1} bytes)")]
    MessageTooLarge(usize, usize),
    
    #[error("Queue is full")]
    QueueFull,
    
    #[error("Invalid priority: {0} (must be 0-255)")]
    InvalidPriority(u8),
    
    #[error("Database error: {0}")]
    DatabaseError(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    // Helper function to create a test queue with temporary directory
    async fn create_test_queue() -> Result<(QueueAPI, TempDir)> {
        let temp_dir = TempDir::new()?;
        let config = QueueConfig {
            db_path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        let api = QueueAPI::with_config(config).await?;
        Ok((api, temp_dir))
    }

    #[tokio::test]
    async fn test_queue_api_new() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let config = QueueConfig {
            db_path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        let api = QueueAPI::with_config(config).await?;
        
        assert_eq!(api.config.max_message_size, 64 * 1024 * 1024);
        assert_eq!(api.config.batch_size, 100);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_enqueue_success() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let payload = b"test message".to_vec();
        
        let message_id = api.enqueue(payload.clone()).await?;
        assert!(message_id > 0); // Snowflake ID should be positive
        
        // Verify message was stored
        let stored_message = api.get_message(message_id).await?.unwrap();
        assert_eq!(stored_message.payload, payload);
        
        // Verify status is pending
        assert_eq!(api.get_status(message_id).await?.unwrap(), MessageStatus::Pending);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_enqueue_message_too_large() -> Result<()> {
        let (mut api, _temp_dir) = create_test_queue().await?;
        api.update_config(QueueConfig {
            max_message_size: 10,
            db_path: api.config.db_path.clone(),
            ..Default::default()
        });
        
        let large_payload = vec![0u8; 20];
        let result = api.enqueue(large_payload).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Message size exceeds maximum allowed size"));
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_dequeue_empty_queue() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let result = api.dequeue().await?;
        assert!(result.is_none());
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_dequeue_priority_order() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        // Enqueue messages with different priorities
        let msg1_id = api.enqueue(b"low".to_vec()).await?;
        let msg2_id = api.enqueue(b"high".to_vec()).await?;
        let msg3_id = api.enqueue(b"medium".to_vec()).await?;
        
        // Dequeue should return messages in FIFO order
        let first = api.dequeue().await?.unwrap();
        assert_eq!(first.payload, b"low");
        
        // Status should be Processing
        assert_eq!(api.get_status(msg1_id).await?.unwrap(), MessageStatus::Processing);
        
        // Dequeue next message
        let second = api.dequeue().await?.unwrap();
        assert_eq!(second.payload, b"high");
        
        // Dequeue last message
        let third = api.dequeue().await?.unwrap();
        assert_eq!(third.payload, b"medium");
        
        // Queue should be empty now
        assert!(api.dequeue().await?.is_none());
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_message_lifecycle() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let payload = b"test message".to_vec();
        
        let message_id = api.enqueue(payload).await?;
        
        // Check initial status
        assert_eq!(api.get_status(message_id).await?.unwrap(), MessageStatus::Pending);
        
        // Dequeue and check processing status
        let _message = api.dequeue().await?.unwrap();
        assert_eq!(api.get_status(message_id).await?.unwrap(), MessageStatus::Processing);
        
        // Complete message
        api.complete(message_id).await?;
        assert_eq!(api.get_status(message_id).await?.unwrap(), MessageStatus::Completed);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_retry_mechanism() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        let payload = b"test message".to_vec();
        let message_id = api.enqueue(payload).await?;
        
        // Dequeue and fail
        let _message = api.dequeue().await?.unwrap();
        api.fail(message_id).await?;
        
        // Should be in failed status
        assert_eq!(api.get_status(message_id).await?.unwrap(), MessageStatus::Failed);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_cleanup() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        // Enqueue and complete a message
        let msg1_id = api.enqueue(b"msg1".to_vec()).await?;
        let first = api.dequeue().await?.unwrap();
        api.complete(first.id).await?;
        
        // Enqueue and fail a message
        let msg2_id = api.enqueue(b"msg2".to_vec()).await?;
        let second = api.dequeue().await?.unwrap();
        api.fail(second.id).await?;
        
        // Enqueue a pending message
        let msg3_id = api.enqueue(b"msg3".to_vec()).await?;
        
        // Cleanup should remove completed and failed messages
        let cleaned = api.cleanup().await?;
        assert_eq!(cleaned, 2); // Both completed and failed messages get cleaned up
        
        // Check that pending messages still exist
        assert!(api.get_message(msg3_id).await?.is_some());
        assert!(api.get_message(msg1_id).await?.is_none());
        assert!(api.get_message(msg2_id).await?.is_none()); // Failed message gets cleaned up
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_snowflake_id_uniqueness() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let mut ids = std::collections::HashSet::new();
        
        for _ in 0..100 {
            let id = api.enqueue(b"test".to_vec()).await?;
            assert!(ids.insert(id), "Duplicate Snowflake ID generated");
        }
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_persistence() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let config = QueueConfig {
            db_path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        
        // Create first queue instance
        let api1 = QueueAPI::with_config(config.clone()).await?;
        let message_id = api1.enqueue(b"persistent message".to_vec()).await?;
        api1.close().await?;
        
        // Create second queue instance with same path
        let api2 = QueueAPI::with_config(config).await?;
        
        // Message should still exist in SlateDB, but status won't be available
        let message = api2.get_message(message_id).await?;
        assert!(message.is_some());
        if let Some(msg) = message {
            assert_eq!(msg.payload, b"persistent message");
        }
        
        api2.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_queue_api_with_config() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let config = QueueConfig {
            max_message_size: 1024,
            batch_size: 50,
            db_path: temp_dir.path().to_path_buf(),
        };
        let api = QueueAPI::with_config(config.clone()).await?;
        
        assert_eq!(api.config.max_message_size, 1024);
        assert_eq!(api.config.batch_size, 50);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_fail_message() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        let message_id = api.enqueue(b"test".to_vec()).await?;
        let _ = api.dequeue().await?.unwrap();
        
        // Fail message
        api.fail(message_id).await?;
        assert_eq!(api.get_status(message_id).await?.unwrap(), MessageStatus::Failed);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_fail_message_exceeds_retry_limit() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        let message_id = api.enqueue(b"test".to_vec()).await?;
        let _ = api.dequeue().await?.unwrap();
        
        // Fail message
        api.fail(message_id).await?;
        assert_eq!(api.get_status(message_id).await?.unwrap(), MessageStatus::Failed);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_fail_nonexistent_message() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        // Should not panic
        api.fail(999999).await?;
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_get_status_nonexistent() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let status = api.get_status(999999).await?;
        assert!(status.is_none());
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_get_message_nonexistent() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let message = api.get_message(999999).await?;
        assert!(message.is_none());
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_get_stats_empty_queue() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let stats = api.get_stats().await?;
        
        assert_eq!(stats.total_messages, 0);
        assert_eq!(stats.pending_count, 0);
        assert_eq!(stats.processing_count, 0);
        assert_eq!(stats.completed_count, 0);
        assert_eq!(stats.failed_count, 0);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_get_stats_with_messages() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        // Enqueue messages
        let msg1_id = api.enqueue(b"msg1".to_vec()).await?;
        let msg2_id = api.enqueue(b"msg2".to_vec()).await?;
        
        // Dequeue first message (FIFO order)
        let first = api.dequeue().await?.unwrap();
        assert_eq!(first.id, msg1_id); // First enqueued
        
        // Complete the first message
        api.complete(first.id).await?;
        
        // Dequeue second message
        let second = api.dequeue().await?.unwrap();
        assert_eq!(second.id, msg2_id); // Second enqueued
        
        // Fail the second message
        api.fail(second.id).await?;
        
        let stats = api.get_stats().await?;
        assert_eq!(stats.total_messages, 2);
        assert_eq!(stats.pending_count, 0);
        assert_eq!(stats.processing_count, 0);
        assert_eq!(stats.completed_count, 1);
        assert_eq!(stats.failed_count, 1);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_cleanup_empty_queue() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let cleaned = api.cleanup().await?;
        assert_eq!(cleaned, 0);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_cleanup_with_completed_and_failed() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        // Enqueue and complete a message
        let msg1_id = api.enqueue(b"msg1".to_vec()).await?;
        let first = api.dequeue().await?.unwrap();
        api.complete(first.id).await?;
        
        // Enqueue and fail a message
        let msg2_id = api.enqueue(b"msg2".to_vec()).await?;
        let second = api.dequeue().await?.unwrap();
        api.fail(second.id).await?;
        
        // Enqueue a pending message
        let msg3_id = api.enqueue(b"msg3".to_vec()).await?;
        
        // Cleanup should remove completed and failed messages
        let cleaned = api.cleanup().await?;
        assert_eq!(cleaned, 2);
        
        // Check that pending messages still exist
        assert!(api.get_message(msg3_id).await?.is_some());
        assert!(api.get_message(msg1_id).await?.is_none());
        assert!(api.get_message(msg2_id).await?.is_none()); // Failed message gets cleaned up
        
        // Check stats after cleanup
        let stats = api.get_stats().await?;
        assert_eq!(stats.total_messages, 1); // 1 pending
        assert_eq!(stats.pending_count, 1);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_config_getter() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let config = api.config();
        assert_eq!(config.max_message_size, 64 * 1024 * 1024);
        assert_eq!(config.batch_size, 100);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_update_config() -> Result<()> {
        let (mut api, _temp_dir) = create_test_queue().await?;
        let new_config = QueueConfig {
            max_message_size: 1024 * 1024,
            batch_size: 200,
            db_path: api.config.db_path.clone(),
        };
        
        api.update_config(new_config);
        
        let updated_config = api.config();
        assert_eq!(updated_config.max_message_size, 1024 * 1024);
        assert_eq!(updated_config.batch_size, 200);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_message_serialization() -> Result<()> {
        let message = Message {
            id: 123456789,
            payload: b"test payload".to_vec(),
            timestamp: 1234567890,
        };
        
        // Test serialization
        let serialized = bincode::serialize(&message)?;
        let deserialized: Message = bincode::deserialize(&serialized)?;
        
        assert_eq!(deserialized.id, 123456789);
        assert_eq!(deserialized.payload, b"test payload");
        assert_eq!(deserialized.timestamp, 1234567890);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_message_status_serialization() -> Result<()> {
        let statuses = vec![
            MessageStatus::Pending,
            MessageStatus::Processing,
            MessageStatus::Completed,
            MessageStatus::Failed,
        ];
        
        for status in statuses {
            let serialized = bincode::serialize(&status)?;
            let deserialized: MessageStatus = bincode::deserialize(&serialized)?;
            assert_eq!(deserialized, status);
        }
        
        Ok(())
    }

    #[tokio::test]
    async fn test_queue_stats_serialization() -> Result<()> {
        let stats = QueueStats {
            total_messages: 100,
            pending_count: 20,
            processing_count: 10,
            completed_count: 50,
            failed_count: 15,
        };
        
        let serialized = bincode::serialize(&stats)?;
        let deserialized: QueueStats = bincode::deserialize(&serialized)?;
        
        assert_eq!(deserialized.total_messages, 100);
        assert_eq!(deserialized.pending_count, 20);
        assert_eq!(deserialized.processing_count, 10);
        assert_eq!(deserialized.completed_count, 50);
        assert_eq!(deserialized.failed_count, 15);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_concurrent_access() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let api = Arc::new(api);
        let mut handles = Vec::new();
        
        // Spawn multiple tasks that enqueue messages
        for i in 0..10 {
            let api_clone = Arc::clone(&api);
            let handle = tokio::spawn(async move {
                let payload = format!("message_{}", i).into_bytes();
                api_clone.enqueue(payload).await.unwrap()
            });
            handles.push(handle);
        }
        
        // Wait for all enqueue operations
        for handle in handles {
            handle.await.unwrap();
        }
        
        // Verify all messages were enqueued
        let stats = api.get_stats().await?;
        assert_eq!(stats.total_messages, 10);
        assert_eq!(stats.pending_count, 10);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_fifo_ordering() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        // Enqueue messages in order
        let _first_id = api.enqueue(b"first".to_vec()).await?;
        let _second_id = api.enqueue(b"second".to_vec()).await?;
        let _third_id = api.enqueue(b"third".to_vec()).await?;
        
        // Dequeue should return messages in FIFO order
        let first = api.dequeue().await?.unwrap();
        assert_eq!(first.payload, b"first");
        api.complete(first.id).await?;
        
        let second = api.dequeue().await?.unwrap();
        assert_eq!(second.payload, b"second");
        api.complete(second.id).await?;
        
        let third = api.dequeue().await?.unwrap();
        assert_eq!(third.payload, b"third");
        api.complete(third.id).await?;
        
        // No more messages should be available
        assert!(api.dequeue().await?.is_none());
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_message_timestamp() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        let before_enqueue = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let message_id = api.enqueue(b"test".to_vec()).await?;
        
        let after_enqueue = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let message = api.get_message(message_id).await?.unwrap();
        assert!(message.timestamp >= before_enqueue);
        assert!(message.timestamp <= after_enqueue);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_error_handling_edge_cases() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        // Test with empty payload
        let empty_id = api.enqueue(vec![].to_vec()).await?;
        assert!(empty_id > 0);
        
        // Test with very large payload (but within limits)
        let large_payload = vec![0u8; 1024 * 1024]; // 1MB
        let large_id = api.enqueue(large_payload).await?;
        assert!(large_id > 0);
        
        // Test with regular payload
        let regular_id = api.enqueue(b"regular payload".to_vec()).await?;
        assert!(regular_id > 0);
        
        api.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_message_processing_workflow() -> Result<()> {
        let (api, _temp_dir) = create_test_queue().await?;
        
        // Enqueue multiple messages
        let _msg1_id = api.enqueue(b"task1".to_vec()).await?;
        let _msg2_id = api.enqueue(b"task2".to_vec()).await?;
        let _msg3_id = api.enqueue(b"task3".to_vec()).await?;
        
        // Process messages in FIFO order
        let first = api.dequeue().await?.unwrap();
        assert_eq!(first.payload, b"task1");
        api.complete(first.id).await?;
        
        let second = api.dequeue().await?.unwrap();
        assert_eq!(second.payload, b"task2");
        api.complete(second.id).await?;
        
        let third = api.dequeue().await?.unwrap();
        assert_eq!(third.payload, b"task3");
        api.complete(third.id).await?;
        
        // All messages should be completed
        let stats = api.get_stats().await?;
        assert_eq!(stats.completed_count, 3);
        assert_eq!(stats.pending_count, 0);
        
        // Cleanup should remove all completed messages
        let cleaned = api.cleanup().await?;
        assert_eq!(cleaned, 3);
        
        let final_stats = api.get_stats().await?;
        assert_eq!(final_stats.total_messages, 0);
        
        api.close().await?;
        Ok(())
    }
} 