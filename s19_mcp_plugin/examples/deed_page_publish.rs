//! Run from examples/deed_page so the normal plugin loader finds its manifest.
//! Publishes a page through the MCP router without a model, checks it, then
//! deletes it with the owner token (set DEED_PAGE_KEEP=1 to keep it for an hour).
use anyhow::{Context, Result, ensure};
use s19_mcp_plugin::load_mcp_router;
use serde_json::{Value, json};
use std::time::Duration;

const PUBLISH: &str = "mcp__deed-page__pages__publish_site";
const GET: &str = "mcp__deed-page__pages__get_site";
const DELETE: &str = "mcp__deed-page__pages__delete_site";

#[tokio::main]
async fn main() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(120), run())
        .await
        .context("deed.page example timed out")?
}

async fn run() -> Result<()> {
    let mut router = load_mcp_router().await?;
    let result = async {
        let tools = router.all_tools();
        for name in [PUBLISH, GET, DELETE] {
            ensure!(
                tools.iter().any(|tool| tool.name == name),
                "Missing {name}; run from s19_mcp_plugin/examples/deed_page and check the connection log"
            );
        }

        let page = "<!doctype html><title>s19</title><h1>Published from the s19 MCP router</h1>";
        let published = call_json(
            &mut router,
            PUBLISH,
            json!({"files": {"index.html": page}, "ttl_seconds": 3600}),
        )
        .await?;
        let slug = published["slug"].as_str().context("publish returned no slug")?;
        println!("Published: {} (expires {})", published["url"], published["expires_at"]);

        let site = call_json(&mut router, GET, json!({"slug": slug})).await?;
        ensure!(site["exists"] == json!(true), "get_site did not find {slug}");
        println!("get_site: {} bytes, version {}", site["site"]["bytes"], site["site"]["version"]);

        if std::env::var("DEED_PAGE_KEEP").is_ok_and(|value| value == "1") {
            println!("Kept; the page expires in one hour.");
            return Ok(());
        }
        // The owner token is only returned on this first publish; it is not printed.
        let token = published["token"].as_str().context("publish returned no token")?;
        let deleted = call_json(&mut router, DELETE, json!({"slug": slug, "token": token})).await?;
        ensure!(deleted["deleted"].as_str() == Some(slug), "delete_site did not confirm {slug}");
        println!("Deleted {slug}");
        Ok(())
    }
    .await;
    router.disconnect_all().await;
    result
}

async fn call_json(
    router: &mut s19_mcp_plugin::router::MCPToolRouter,
    name: &str,
    arguments: Value,
) -> Result<Value> {
    let text = router.call(name, arguments).await?;
    let value: Value =
        serde_json::from_str(&text).with_context(|| format!("{name} returned: {text}"))?;
    ensure!(
        value["ok"].as_bool() != Some(false),
        "{name} failed: {text}"
    );
    Ok(value)
}
