// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use std::collections::{HashMap, HashSet};

use dynamo_kv_router::protocols::{ActiveLoad, DpRank};
use dynamo_runtime::component::Endpoint;
use dynamo_runtime::traits::DistributedRuntimeProvider;
use dynamo_runtime::transports::event_plane::EventPublisher;

use crate::kv_router::KV_METRICS_SUBJECT;

#[derive(Debug, Clone, Default, PartialEq)]
struct WorkerMetrics {
    active_decode_blocks: Option<u64>,
    kv_used_blocks: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct MetricsState {
    /// Per-rank metrics. Each rank maintains its own metrics.
    ///
    /// Under attention-DP configurations (e.g., TensorRT-LLM reporting multiple
    /// ranks from one process), this map ensures metrics from all ranks are
    /// preserved when published within the debounce window.
    metrics: HashMap<DpRank, WorkerMetrics>,
    /// Ranks that have been updated since the last publish.
    ///
    /// Dirty tracking ensures all modified ranks are flushed when the debounce
    /// timer fires, preventing rank coalescing that would drop metrics.
    dirty_ranks: HashSet<DpRank>,
}

pub struct WorkerMetricsPublisher {
    tx: tokio::sync::watch::Sender<MetricsState>,
    rx: tokio::sync::watch::Receiver<MetricsState>,
}

impl WorkerMetricsPublisher {
    pub fn new() -> Result<Self> {
        let (tx, rx) = tokio::sync::watch::channel(MetricsState::default());
        Ok(Self { tx, rx })
    }

    pub fn publish(
        &self,
        dp_rank: Option<DpRank>,
        active_decode_blocks: Option<u64>,
        kv_used_blocks: Option<u64>,
    ) -> Result<()> {
        if active_decode_blocks.is_none() && kv_used_blocks.is_none() {
            anyhow::bail!("worker metrics publish requires at least one load metric");
        }

        let rank = dp_rank.unwrap_or(0);
        let metrics = WorkerMetrics {
            active_decode_blocks,
            kv_used_blocks,
        };
        tracing::trace!(
            "Publish metrics: dp_rank={}, active_decode_blocks={:?}, kv_used_blocks={:?}",
            rank,
            metrics.active_decode_blocks,
            metrics.kv_used_blocks
        );
        
        self.tx.send_modify(|state| {
            state.metrics.insert(rank, metrics);
            state.dirty_ranks.insert(rank);
        });
        
        Ok(())
    }

    pub async fn create_endpoint(&self, endpoint: Endpoint) -> Result<()> {
        let worker_id = endpoint.drt().connection_id();
        let event_publisher = EventPublisher::for_endpoint(&endpoint, KV_METRICS_SUBJECT).await?;
        self.start_metrics_publishing(event_publisher, worker_id);
        Ok(())
    }

    pub(super) fn start_metrics_publishing(&self, event_publisher: EventPublisher, worker_id: u64) {
        let metrics_rx = self.rx.clone();

        tokio::spawn(async move {
            let mut rx = metrics_rx;
            let mut last_state = MetricsState::default();
            let publish_timer = tokio::time::sleep(tokio::time::Duration::ZERO);
            tokio::pin!(publish_timer);

            loop {
                tokio::select! {
                    result = rx.changed() => {
                        if result.is_err() {
                            tracing::debug!(
                                "Metrics publisher sender dropped, stopping event-plane background task"
                            );
                            break;
                        }

                        let state = rx.borrow_and_update().clone();
                        
                        // Check if any ranks have changed
                        let mut has_changes = false;
                        for &rank in &state.dirty_ranks {
                            if state.metrics.get(&rank) != last_state.metrics.get(&rank) {
                                has_changes = true;
                                break;
                            }
                        }
                        
                        if !has_changes {
                            continue;
                        }

                        last_state = state;
                        publish_timer.as_mut().reset(
                            tokio::time::Instant::now()
                                + tokio::time::Duration::from_millis(1)
                        );
                    }
                    _ = &mut publish_timer, if !last_state.dirty_ranks.is_empty() => {
                        // Publish all dirty ranks
                        for &rank in &last_state.dirty_ranks {
                            if let Some(metrics) = last_state.metrics.get(&rank) {
                                let active_load = ActiveLoad {
                                    worker_id,
                                    dp_rank: rank,
                                    active_decode_blocks: metrics.active_decode_blocks,
                                    active_prefill_tokens: None,
                                    kv_used_blocks: metrics.kv_used_blocks,
                                };

                                if let Err(e) = event_publisher.publish(&active_load).await {
                                    tracing::warn!("Failed to publish metrics for rank {}: {}", rank, e);
                                }
                            }
                        }
                        
                        // Clear dirty ranks after publishing
                        last_state.dirty_ranks.clear();
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Mock event publisher that records published ActiveLoad events
    struct MockEventPublisher {
        published: Arc<Mutex<Vec<ActiveLoad>>>,
    }

    impl MockEventPublisher {
        fn new() -> (Self, Arc<Mutex<Vec<ActiveLoad>>>) {
            let published = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    published: published.clone(),
                },
                published,
            )
        }

        async fn publish(&self, load: &ActiveLoad) -> Result<()> {
            self.published.lock().await.push(load.clone());
            Ok(())
        }
    }

    /// Helper to create a mock EventPublisher compatible with start_metrics_publishing
    fn create_mock_publisher() -> (EventPublisher, Arc<Mutex<Vec<ActiveLoad>>>) {
        // Create a mock that collects published events
        let published = Arc::new(Mutex::new(Vec::new()));
        let published_clone = published.clone();
        
        // We need to create a real EventPublisher for testing
        // Since we can't easily mock EventPublisher, we'll test the publish logic directly
        // by inspecting the watch channel state
        unimplemented!("EventPublisher mocking requires runtime setup")
    }

    #[tokio::test]
    async fn test_single_rank_publish() {
        let publisher = WorkerMetricsPublisher::new().unwrap();
        
        // Publish metrics for rank 0
        publisher.publish(Some(0), Some(100), Some(50)).unwrap();
        
        // Verify state was updated
        let state = publisher.rx.borrow().clone();
        assert_eq!(state.metrics.len(), 1);
        assert_eq!(state.dirty_ranks.len(), 1);
        assert!(state.dirty_ranks.contains(&0));
        
        let metrics = state.metrics.get(&0).unwrap();
        assert_eq!(metrics.active_decode_blocks, Some(100));
        assert_eq!(metrics.kv_used_blocks, Some(50));
    }

    #[tokio::test]
    async fn test_multi_rank_publish_within_debounce_window() {
        let publisher = WorkerMetricsPublisher::new().unwrap();
        
        // Publish metrics for multiple ranks in quick succession (simulating attention-DP)
        publisher.publish(Some(0), Some(100), Some(50)).unwrap();
        publisher.publish(Some(1), Some(200), Some(75)).unwrap();
        publisher.publish(Some(2), Some(150), Some(60)).unwrap();
        
        // Verify all ranks are preserved in the state
        let state = publisher.rx.borrow().clone();
        assert_eq!(state.metrics.len(), 3, "All three ranks should be stored");
        assert_eq!(state.dirty_ranks.len(), 3, "All three ranks should be dirty");
        
        // Verify rank 0 metrics
        assert!(state.dirty_ranks.contains(&0));
        let metrics0 = state.metrics.get(&0).unwrap();
        assert_eq!(metrics0.active_decode_blocks, Some(100));
        assert_eq!(metrics0.kv_used_blocks, Some(50));
        
        // Verify rank 1 metrics
        assert!(state.dirty_ranks.contains(&1));
        let metrics1 = state.metrics.get(&1).unwrap();
        assert_eq!(metrics1.active_decode_blocks, Some(200));
        assert_eq!(metrics1.kv_used_blocks, Some(75));
        
        // Verify rank 2 metrics
        assert!(state.dirty_ranks.contains(&2));
        let metrics2 = state.metrics.get(&2).unwrap();
        assert_eq!(metrics2.active_decode_blocks, Some(150));
        assert_eq!(metrics2.kv_used_blocks, Some(60));
    }

    #[tokio::test]
    async fn test_rank_update_marks_dirty() {
        let publisher = WorkerMetricsPublisher::new().unwrap();
        
        // Initial publish for rank 0
        publisher.publish(Some(0), Some(100), Some(50)).unwrap();
        let state1 = publisher.rx.borrow().clone();
        assert_eq!(state1.dirty_ranks.len(), 1);
        
        // Manually clear dirty ranks (simulating a publish cycle)
        publisher.tx.send_modify(|state| {
            state.dirty_ranks.clear();
        });
        
        let state2 = publisher.rx.borrow().clone();
        assert_eq!(state2.dirty_ranks.len(), 0);
        
        // Update the same rank with new metrics
        publisher.publish(Some(0), Some(120), Some(55)).unwrap();
        
        // Verify rank is marked dirty again
        let state3 = publisher.rx.borrow().clone();
        assert_eq!(state3.dirty_ranks.len(), 1);
        assert!(state3.dirty_ranks.contains(&0));
        
        let metrics = state3.metrics.get(&0).unwrap();
        assert_eq!(metrics.active_decode_blocks, Some(120));
        assert_eq!(metrics.kv_used_blocks, Some(55));
    }

    #[tokio::test]
    async fn test_default_rank_zero() {
        let publisher = WorkerMetricsPublisher::new().unwrap();
        
        // Publish without specifying rank (should default to 0)
        publisher.publish(None, Some(100), Some(50)).unwrap();
        
        let state = publisher.rx.borrow().clone();
        assert_eq!(state.metrics.len(), 1);
        assert!(state.metrics.contains_key(&0));
        assert!(state.dirty_ranks.contains(&0));
    }

    #[tokio::test]
    async fn test_publish_requires_at_least_one_metric() {
        let publisher = WorkerMetricsPublisher::new().unwrap();
        
        // Publishing without any metrics should fail
        let result = publisher.publish(Some(0), None, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("at least one load metric"));
    }

    #[tokio::test]
    async fn test_publish_with_only_active_decode_blocks() {
        let publisher = WorkerMetricsPublisher::new().unwrap();
        
        publisher.publish(Some(0), Some(100), None).unwrap();
        
        let state = publisher.rx.borrow().clone();
        let metrics = state.metrics.get(&0).unwrap();
        assert_eq!(metrics.active_decode_blocks, Some(100));
        assert_eq!(metrics.kv_used_blocks, None);
    }

    #[tokio::test]
    async fn test_publish_with_only_kv_used_blocks() {
        let publisher = WorkerMetricsPublisher::new().unwrap();
        
        publisher.publish(Some(0), None, Some(50)).unwrap();
        
        let state = publisher.rx.borrow().clone();
        let metrics = state.metrics.get(&0).unwrap();
        assert_eq!(metrics.active_decode_blocks, None);
        assert_eq!(metrics.kv_used_blocks, Some(50));
    }

    #[tokio::test]
    async fn test_concurrent_rank_publishes() {
        let publisher = Arc::new(WorkerMetricsPublisher::new().unwrap());
        
        // Simulate concurrent publishes from different ranks
        let handles: Vec<_> = (0..10)
            .map(|rank| {
                let p = publisher.clone();
                tokio::spawn(async move {
                    p.publish(Some(rank), Some(rank as u64 * 10), Some(rank as u64 * 5))
                        .unwrap();
                })
            })
            .collect();
        
        // Wait for all publishes
        for handle in handles {
            handle.await.unwrap();
        }
        
        // Verify all ranks are present
        let state = publisher.rx.borrow().clone();
        assert_eq!(state.metrics.len(), 10, "All 10 ranks should be stored");
        assert_eq!(state.dirty_ranks.len(), 10, "All 10 ranks should be dirty");
        
        for rank in 0..10 {
            assert!(state.metrics.contains_key(&rank));
            assert!(state.dirty_ranks.contains(&rank));
            let metrics = state.metrics.get(&rank).unwrap();
            assert_eq!(metrics.active_decode_blocks, Some(rank as u64 * 10));
            assert_eq!(metrics.kv_used_blocks, Some(rank as u64 * 5));
        }
    }
}
