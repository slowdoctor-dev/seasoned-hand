pub fn resolve_database_url() -> String {
    std::env::var("SH_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "sqlite:./data/seasoned-hand.db".to_string())
}
