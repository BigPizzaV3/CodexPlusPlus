pub const VERSION: &str = "1.1.6.1";

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn exposes_workspace_version() {
        assert!(!VERSION.is_empty());
    }
}
