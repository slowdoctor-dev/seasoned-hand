use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct GaiaFixture {
    pub title: String,
    pub briefing: String,
    pub expected_in_final_message: Vec<String>,
    pub max_steps: u32,
    pub cost_cap_cents: u32,
}

pub fn load_all(dir: &str) -> std::io::Result<Vec<GaiaFixture>> {
    let mut paths = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();

    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let raw = std::fs::read_to_string(&p)?;
        let f: GaiaFixture = serde_json::from_str(&raw).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {e}", p.display()),
            )
        })?;
        out.push(f);
    }
    Ok(out)
}
