pub const VERSION: &str = "1.2.2";

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn exposes_workspace_version() {
        assert_eq!(VERSION, "1.2.2");
    }
}
