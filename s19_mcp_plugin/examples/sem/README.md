# Local code lookup with Sem

This example loads a local [Sem](https://github.com/Ataraxy-Labs/sem) stdio
server through s19's plugin loader, then calls `sem_find` through its MCP router.
It prints source context without running an agent or requiring model credentials.

Install Sem on `PATH` (`sem --version`; tested with 0.27.0) and the Rust toolchain.
From the repository root:

```sh
cd s19_mcp_plugin/examples/sem
SEM_REPO=/absolute/path/to/checkout cargo run -p s19_mcp_plugin --example sem_code -- parse_config
```

Choose an existing checkout and replace `parse_config` with a symbol in it.
`SEM_REPO` is inherited by the subprocess; the example rejects a missing or
relative path so the plugin directory is not indexed by accident. Connection
and lookup timeouts are separate, and the router disconnects after the call.

Sem may write a local index. The path is a repository selector, not a sandbox.
This direct example invokes only `sem_find`. Loading the manifest into the
chapter's agent instead exposes all discovered tools, including validation
capabilities that can execute commands: review permissions before doing that.
Any source returned to an agent goes to its configured model.

To check the example manifest without starting Sem, run from the repository root:

```sh
cargo test -p s19_mcp_plugin --example sem_code
```
