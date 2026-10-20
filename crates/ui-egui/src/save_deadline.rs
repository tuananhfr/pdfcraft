//! Save deadlines in seconds supplied by the caller's clock.

pub(crate) const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone, Copy)]
pub(crate) struct SaveDeadline {
    started: f64,
}

impl SaveDeadline {
    pub(crate) fn new(now: f64) -> Self {
        Self { started: now }
    }

    pub(crate) fn expired(self, now: f64) -> bool {
        now - self.started >= TIMEOUT.as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_fires_at_sixty_seconds() {
        let deadline = SaveDeadline::new(12.0);
        assert!(!deadline.expired(71.999));
        assert!(deadline.expired(72.0));
        assert!(deadline.expired(90.0));
    }
}
