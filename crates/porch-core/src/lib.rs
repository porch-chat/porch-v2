//! Client engine boundary. Networking, sync and caching arrive in later phases.
pub fn protocol_version() -> u16 {
    porch_protocol::PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    #[test]
    fn uses_shared_protocol() {
        assert_eq!(super::protocol_version(), porch_protocol::PROTOCOL_VERSION);
    }
}
