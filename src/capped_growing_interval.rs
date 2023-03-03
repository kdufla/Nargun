use std::time::Duration;
use tokio::time::sleep;
pub struct CappedGrowingInterval {
    growth_formula: fn(f64) -> f64,
    iter_count: usize,
    max: f64,
}

impl CappedGrowingInterval {
    pub fn new(max: f64, growth_formula: fn(f64) -> f64) -> Self {
        Self {
            growth_formula,
            iter_count: 0,
            max,
        }
    }

    pub async fn tick(&mut self) {
        let sleep_duration = [(self.growth_formula)(self.iter_count as f64), self.max]
            .into_iter()
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();

        sleep(Duration::from_secs_f64(sleep_duration)).await;

        self.iter_count += 1;
    }
}
