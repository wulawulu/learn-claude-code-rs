# deed.page MCP example

This opt-in example loads the keyless [deed.page](https://deed.page/docs) MCP server through s19's existing plugin discovery. deed.page publishes static HTML to a public URL; the first anonymous publish returns an owner token that is needed to update or delete the page. The pinned `mcp-remote@0.14.3` bridge connects the stdio client to `https://deed.page/mcp` using Streamable HTTP and sends `User-Agent: learn-claude-code-rs/0.1.0`. The root plugin manifest stays unchanged.

The example and this README come from the deed.page team (Avatar 8 LLC). The same server is also packaged as an MIT agent skill: <https://github.com/f-o-x11/deed-page-skill>.

Install Rust (stable) and Node.js 22 or newer, including npm/npx. No model credentials or API key are needed for the scripted run. From the repository root:

```bash
cd s19_mcp_plugin/examples/deed_page
cargo run -p s19_mcp_plugin --example deed_page_publish
```

Run from this directory so the loader discovers its `.claude-plugin/plugin.json`. On first use, npx downloads the pinned bridge from npm. Both need network access.

The example does not call a model. It uses `load_mcp_router` and `MCPToolRouter::call` directly, so you can see the namespaced tool names (`mcp__deed-page__pages__publish_site`, `..._get_site`, `..._delete_site`) and the raw JSON results that the agent loop would otherwise pass to the model. It:

1. checks that the three tools were discovered,
2. publishes a one-file page with `ttl_seconds: 3600` and prints its URL,
3. reads it back with `get_site`,
4. deletes it with the owner token from step 2. The token is kept in memory and never printed.

Set `DEED_PAGE_KEEP=1` to skip step 4. The page then expires after one hour, and no token is shown, so it cannot be deleted early.

For the chapter's full interactive UI, run `cargo run -p s19_mcp_plugin` here, choose **default**, then ask:

> Use publish_site to publish a one-page HTML summary of what s19 adds, with ttl_seconds 600, and give me the URL.

Under the chapter's existing classifier `publish_site` is a write capability, so `default` mode asks before each call; choose **allow once**. Published pages are public, so do not publish secrets. Anonymous pages are temporary and show a small "Published with deed.page" badge; see the [docs](https://deed.page/docs) for limits.
