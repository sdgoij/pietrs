//! A verdict collector for corpus runners.

/// Collects one `OK:` or `FAIL:` line per case and counts the failures.
#[derive(Debug, Default)]
pub struct Checks {
    total: usize,
    failures: usize,
}

impl Checks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prints the verdict for one case.
    pub fn check(&mut self, name: &str, result: Result<(), String>) {
        self.total += 1;
        match result {
            Ok(()) => println!("OK: {name}"),
            Err(reason) => {
                self.failures += 1;
                println!("FAIL: {name}: {reason}");
            }
        }
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn failures(&self) -> usize {
        self.failures
    }

    pub fn passed(&self) -> bool {
        self.failures == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_are_counted() {
        let mut checks = Checks::new();
        checks.check("first", Ok(()));
        checks.check("second", Err("deliberate".to_owned()));
        assert_eq!(checks.total(), 2);
        assert_eq!(checks.failures(), 1);
        assert!(!checks.passed());
    }

    #[test]
    fn no_cases_passes_by_default() {
        assert!(Checks::new().passed());
    }
}
