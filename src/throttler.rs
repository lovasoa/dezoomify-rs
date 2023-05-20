use std::time::{Duration, Instant};

pub struct Throttler {
    last_update: Instant,
    min_interval: Duration,
}

impl Throttler {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            last_update: Instant::now(),
            min_interval,
        }
    }

    pub async fn wait(&mut self) {
        if self.min_interval.is_zero() {
            return;
        }
        let now = Instant::now();
        let next_allowed = self.last_update + self.min_interval;
        self.last_update = now;
        let sleep_time = next_allowed.saturating_duration_since(now);
        if !sleep_time.is_zero() {
            tokio::time::sleep(sleep_time).await;
        }
    }
}
