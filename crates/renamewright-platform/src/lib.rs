#![forbid(unsafe_code)]

/// Filesystem mutation remains unavailable during the planning milestone.
#[must_use]
pub const fn mutation_is_enabled() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::mutation_is_enabled;

    #[test]
    fn planning_milestone_is_read_only() {
        assert!(!mutation_is_enabled());
    }
}
