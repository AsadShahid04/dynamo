// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use std::collections::HashMap;

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

/// Per-rank metrics stored in the watch channel.
///
/// Under attention-DP configurations (e.g., TensorRT-LLM reporting multiple
/// ranks from one process), this map ensures metrics from all ranks are
/// preserved when published within the debounce window.
type MetricsMap = HashMap<DpRank, WorkerMetrics>;

pub struct WorkerMetricsPublisher {
    tx: tokio::sync::watch::Sender<MetricsMap>,
    rx: tokio::sync::watch::Receiver<MetricsMap>,
}

impl WorkerMetricsPublisher {
    pub fn new() -> Result<Self> {
        let (tx, rx) = tokio::sync::watch::channel(MetricsMap::default());
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

        self.tx.send_modify(|map| {
            map.insert(rank, metrics);
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
        let event_publisher = std::sync::Arc::new(event_publisher);

        tokio::spawn(async move {
            Self::run_publishing_loop(metrics_rx, worker_id, move |load| {
                let publisher = event_publisher.clone();
                Box::pin(async move {
                    if let Err(e) = publisher.publish(&load).await {
                        tracing::warn!(
                            "Failed to publish metrics for rank {}: {}",
                            load.dp_rank,
                            e
                        );
                    }
                })
            })
            .await;
        });
    }

    async fn run_publishing_loop<F, Fut>(
        mut rx: tokio::sync::watch::Receiver<MetricsMap>,
        worker_id: u64,
        mut publish_fn: F,
    ) where
        F: FnMut(ActiveLoad) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let mut last_metrics = MetricsMap::default();
        let mut pending: HashMap<DpRank, WorkerMetrics> = HashMap::new();
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

                    let current_metrics = rx.borrow_and_update().clone();

                    // Diff against last_metrics to find changed ranks
                    let mut any_changed = false;
                    for (rank, metrics) in &current_metrics {
                        if last_metrics.get(rank) != Some(metrics) {
                            pending.insert(*rank, metrics.clone());
                            last_metrics.insert(*rank, metrics.clone());
                            any_changed = true;
                        }
                    }

                    if any_changed {
                        publish_timer.as_mut().reset(
                            tokio::time::Instant::now()
                                + tokio::time::Duration::from_millis(1)
                        );
                    }
                }
                _ = &mut publish_timer, if !pending.is_empty() => {
                    // Publish all pending ranks
                    for (rank, metrics) in pending.drain() {
                        let active_load = ActiveLoad {
                            worker_id,
                            dp_rank: rank,
                            active_decode_blocks: metrics.active_decode_blocks,
                            active_prefill_tokens: None,
                            kv_used_blocks: metrics.kv_used_blocks,
                        };

                        publish_fn(active_load).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_single_rank_publish() {
        let publisher = WorkerMetricsPublisher::new().unwrap();

        publisher.publish(Some(0), Some(100), Some(50)).unwrap();

        let map = publisher.rx.borrow().clone();
        assert_eq!(map.len(), 1);

        let metrics = map.get(&0).unwrap();
        assert_eq!(metrics.active_decode_blocks, Some(100));
        assert_eq!(metrics.kv_used_blocks, Some(50));
    }

    #[tokio::test]
    async fn test_multi_rank_debounce_flush() {
        tokio::time::pause();

        let publisher = WorkerMetricsPublisher::new().unwrap();
        let published = Arc::new(Mutex::new(Vec::new()));
        let published_clone = published.clone();

        let rx = publisher.rx.clone();

        tokio::spawn(async move {
            WorkerMetricsPublisher::run_publishing_loop(
                rx,
                123, // worker_id
                move |load: ActiveLoad| {
                    let pub_clone = published_clone.clone();
                    Box::pin(async move {
                        pub_clone.lock().await.push(load);
                    })
                },
            )
            .await;
        });

        // Publish multiple ranks back-to-back (within debounce window)
        publisher.publish(Some(0), Some(100), Some(50)).unwrap();
        publisher.publish(Some(1), Some(200), Some(75)).unwrap();
        publisher.publish(Some(2), Some(150), Some(60)).unwrap();

        // Give rx.changed() time to trigger and the select loop to process
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(tokio::time::Duration::from_micros(1)).await;

        // Advance past debounce window (1ms)
        tokio::time::advance(tokio::time::Duration::from_millis(2)).await;

        // Give the publishing task time to execute
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(tokio::time::Duration::from_micros(1)).await;

        // All three ranks should have been published
        let events = published.lock().await;
        assert_eq!(
            events.len(),
            3,
            "Expected 3 published events, got {}",
            events.len()
        );

        // Verify rank 0
        let event0 = events.iter().find(|e| e.dp_rank == 0).unwrap();
        assert_eq!(event0.worker_id, 123);
        assert_eq!(event0.active_decode_blocks, Some(100));
        assert_eq!(event0.kv_used_blocks, Some(50));

        // Verify rank 1
        let event1 = events.iter().find(|e| e.dp_rank == 1).unwrap();
        assert_eq!(event1.worker_id, 123);
        assert_eq!(event1.active_decode_blocks, Some(200));
        assert_eq!(event1.kv_used_blocks, Some(75));

        // Verify rank 2
        let event2 = events.iter().find(|e| e.dp_rank == 2).unwrap();
        assert_eq!(event2.worker_id, 123);
        assert_eq!(event2.active_decode_blocks, Some(150));
        assert_eq!(event2.kv_used_blocks, Some(60));
    }

    #[tokio::test]
    async fn test_single_rank_update_only_publishes_that_rank() {
        tokio::time::pause();

        let publisher = WorkerMetricsPublisher::new().unwrap();
        let published = Arc::new(Mutex::new(Vec::new()));
        let published_clone = published.clone();

        let rx = publisher.rx.clone();

        tokio::spawn(async move {
            WorkerMetricsPublisher::run_publishing_loop(rx, 456, move |load: ActiveLoad| {
                let pub_clone = published_clone.clone();
                Box::pin(async move {
                    pub_clone.lock().await.push(load);
                })
            })
            .await;
        });

        // Initial publish of three ranks
        publisher.publish(Some(0), Some(100), Some(50)).unwrap();
        publisher.publish(Some(1), Some(200), Some(75)).unwrap();
        publisher.publish(Some(2), Some(150), Some(60)).unwrap();

        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(tokio::time::Duration::from_micros(1)).await;
        tokio::time::advance(tokio::time::Duration::from_millis(2)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(tokio::time::Duration::from_micros(1)).await;

        {
            let events = published.lock().await;
            assert_eq!(events.len(), 3, "Initial flush should publish 3 ranks");
        }

        // Clear published events
        published.lock().await.clear();

        // Update only rank 1
        publisher.publish(Some(1), Some(250), Some(80)).unwrap();

        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(tokio::time::Duration::from_micros(1)).await;
        tokio::time::advance(tokio::time::Duration::from_millis(2)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(tokio::time::Duration::from_micros(1)).await;

        // Only rank 1 should be published
        let events = published.lock().await;
        assert_eq!(
            events.len(),
            1,
            "Only the updated rank should be published, got {}",
            events.len()
        );
        assert_eq!(events[0].dp_rank, 1);
        assert_eq!(events[0].active_decode_blocks, Some(250));
        assert_eq!(events[0].kv_used_blocks, Some(80));
    }

    #[tokio::test]
    async fn test_default_rank_zero() {
        let publisher = WorkerMetricsPublisher::new().unwrap();

        publisher.publish(None, Some(100), Some(50)).unwrap();

        let map = publisher.rx.borrow().clone();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&0));
    }

    #[tokio::test]
    async fn test_publish_requires_at_least_one_metric() {
        let publisher = WorkerMetricsPublisher::new().unwrap();

        let result = publisher.publish(Some(0), None, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("at least one load metric")
        );
    }

    #[tokio::test]
    async fn test_publish_with_only_active_decode_blocks() {
        let publisher = WorkerMetricsPublisher::new().unwrap();

        publisher.publish(Some(0), Some(100), None).unwrap();

        let map = publisher.rx.borrow().clone();
        let metrics = map.get(&0).unwrap();
        assert_eq!(metrics.active_decode_blocks, Some(100));
        assert_eq!(metrics.kv_used_blocks, None);
    }

    #[tokio::test]
    async fn test_publish_with_only_kv_used_blocks() {
        let publisher = WorkerMetricsPublisher::new().unwrap();

        publisher.publish(Some(0), None, Some(50)).unwrap();

        let map = publisher.rx.borrow().clone();
        let metrics = map.get(&0).unwrap();
        assert_eq!(metrics.active_decode_blocks, None);
        assert_eq!(metrics.kv_used_blocks, Some(50));
    }
}
