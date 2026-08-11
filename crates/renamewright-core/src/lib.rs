#![forbid(unsafe_code)]

/// Version of the frontend/backend planning protocol.
pub const PROTOCOL_VERSION: u16 = 1;

#[cfg(test)]
mod tests {
    use super::PROTOCOL_VERSION;

    #[test]
    fn protocol_starts_at_version_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}
