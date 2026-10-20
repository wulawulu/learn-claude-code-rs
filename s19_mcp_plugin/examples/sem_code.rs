//! Run from examples/sem with SEM_REPO set to a trusted checkout.
use anyhow::{Context, Result, ensure};
use s19_mcp_plugin::load_mcp_router;
use serde_json::json;
use std::{path::Path, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let repo = std::env::var("SEM_REPO").context("set SEM_REPO to an absolute checkout path")?;
    ensure!(
        Path::new(&repo).is_absolute() && Path::new(&repo).is_dir(),
        "SEM_REPO must be an existing absolute directory"
    );
    let query = std::env::args()
        .nth(1)
        .context("pass a symbol name to look up")?;
    let mut router = tokio::time::timeout(Duration::from_secs(30), load_mcp_router())
        .await
        .context("MCP connection timed out")??;
    let result = tokio::time::timeout(
        Duration::from_secs(120),
        router.call(
            "mcp__sem-code__code__sem_find",
            json!({"query": query, "mode": "context", "token_budget": 1200}),
        ),
    )
    .await;
    router.disconnect_all().await;
    println!("{}", result.context("code lookup timed out")??);
    Ok(())
}

#[cfg(test)]
mod tests {
    use s19_mcp_plugin::plugin::PluginManifest;

    #[test]
    fn manifest_uses_local_stdio_without_credentials() {
        let manifest: PluginManifest =
            serde_json::from_str(include_str!("sem/.claude-plugin/plugin.json")).unwrap();
        assert_eq!(manifest.name, "sem-code");
        let server = &manifest.mcp_servers["code"];
        assert_eq!(server.command, "sem");
        assert_eq!(server.args, ["mcp"]);
        assert!(server.env.is_empty());
    }
}
