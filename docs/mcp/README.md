# SolarCraft MCP server

`solarcraft-mcp` exposes SolarCraft's design and simulation operations to AI clients over the [Model Context Protocol](https://modelcontextprotocol.io/specification/latest). It uses the official Rust SDK (`rmcp` 3.9, which negotiates protocol revisions 2025-11-25 and 2026-07-28) over **stdio**. It calls the same `solarcraft-app` functions as the CLI and the desktop apps, so results are identical everywhere.

## Run

```bash
cargo build --release -p solarcraft-mcp
target/release/solarcraft-mcp --workspace /path/to/projects
```

### Client configuration (generic `mcpServers` JSON)

```json
{
  "mcpServers": {
    "solarcraft": {
      "command": "/absolute/path/to/solarcraft-mcp",
      "args": ["--workspace", "/absolute/path/to/projects"]
    }
  }
}
```

## Tools

| Tool | Side effects | Notes |
|---|---|---|
| `list_projects` | none | `.solarcraft` files in the workspace |
| `create_project` | creates a file | Easy mode; derives tilt, inverter and strings |
| `get_project` | none | Every field is `{value, origin}`, where origin is `user`, `derived` or `default` |
| `set_fields` | saves (undoable) | JSON-pointer edits in one transaction; `dry_run: true` previews without saving |
| `set_mode` | saves | `easy` ↔ `detailed`; never changes values |
| `validate` | none | Issues with stable `code`, `severity` and `pointer` |
| `fetch_weather_pvgis` | **network** + saves | Requires `confirm_network: true`; sends only lat/lon to PVGIS (EC JRC) |
| `import_weather` | saves | EPW or saved PVGIS JSON from inside the workspace |
| `set_roof` | saves (undoable) | Roof outline + obstacles in roof-plane metres; places modules with setback/clearance; Easy mode sizes the system from the modules that fit |
| `use_demo_weather` | saves | Offline clear-sky demo data (labelled; overestimates yield) |
| `export_report` | creates a file | PDF or DOCX in en/es/de/pt-BR inside the workspace; refuses to overwrite unless `overwrite: true` |
| `add_module` | saves | Datasheet → CEC 6-parameter fit, with fit-quality report |
| `simulate` | stores results | Annual hourly simulation |
| `explain` | none | Irradiation and energy loss waterfalls, monthly table |
| `get_results` | none | Annual/monthly, plus hourly paged at 744 rows |
| `undo`, `history` | undo saves | Change log stored in the project file |

Units are SI. Angles are in degrees; azimuth is measured clockwise from north (180 = south). Hourly timestamps are UTC Unix seconds at interval midpoints.

## Safety model

- **Workspace jail.** Paths must be relative, without `..`, and resolve inside `--workspace` after canonicalisation. Project files must end in `.solarcraft`.
- **No command execution.** The server cannot run programs or read files outside the workspace.
- **Explicit network consent.** The only outbound call (PVGIS) needs `confirm_network: true`.
- **Reversible edits.** Every saved change is in the project's change log (`history`, `undo`).
- **Structured errors.** Domain errors come back as tool results with `isError: true` and `{code, message}`. Invalid parameters (bad paths, missing consent) are JSON-RPC errors.

Remote (Streamable HTTP + OAuth) operation is planned as an optional deployment (feature row MCP-004). It is disabled by default.

Tested by `crates/mcp/tests/stdio.rs`, which drives the real binary over JSON-RPC.
