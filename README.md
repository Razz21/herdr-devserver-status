# herdr-devserver-status

Detects frontend dev servers running inside [herdr](https://herdr.dev) panes and reports status — starting, running, building, error — to the herdr sidebar.

## What it does

Background daemon watches open herdr panes. On a confirmed dev server process, reports:

- **Agent state** — `working` / `idle` / `blocked`
- **Display metadata** — label (e.g. "Vite (dev server)") + per-tool tokens (URL, port, error flag)

No project-side setup — detection runs off process list and pane output.

## Supported frameworks

Frameworks are YAML specs. Vite, Next.js, Nuxt, Astro ship as seed defaults on first run. See [Extending](#extending).

## Install

```sh
herdr plugin install Razz21/herdr-devserver-status
```

Downloads a prebuilt binary matching your platform when a release exists for the declared version, verifies SHA-256, falls back to `cargo build --release` on any miss.

## Requirements

- herdr ≥ 0.7.0
- Rust 1.97+ (source builds only)
- macOS, Linux (`x86_64`, `aarch64`)

## Build from source

```sh
cargo build --release
```

`target/release/herdr-devserver-status daemon`

## Logging

stderr always; file additionally when a location resolves. Default level `Info` — startup, detection, state changes, warnings, errors. Raw process/pane responses and per-tick polling are `Debug`, off by default.

## Environment variables

| Variable                  | Default                                               | Purpose                                                                                  |
| ------------------------- | ----------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `HERDR_PLUGIN_CONFIG_DIR` | cwd (warns)                                           | Plugin config root — herdr-set for a live install. Holds `frameworks/` and the log file. |
| `HERDR_BIN_PATH`          | `herdr` on `PATH`                                     | `herdr` binary the daemon shells out to.                                                 |
| `HDS_LOG_LEVEL`           | `info`                                                | `error` \| `warn` \| `info` \| `debug`. Unrecognized → `info`.                           |
| `HDS_LOG_PATH`            | `$HERDR_PLUGIN_CONFIG_DIR/herdr-devserver-status.log` | Log file override.                                                                       |

## How it works

1. **Discovery** (`daemon/discovery.rs`) polls `herdr pane list` every 2s against every loaded spec.
2. **Confirmation**: resolve argv entries, match bin-path regex, confirm resolved script's owning package (`tools/pkg_lookup.rs`).
3. **Worker** (`daemon/worker.rs`) per confirmed pane: blocks on `herdr pane wait-output`, reports state/metadata changes only.
4. On process death or pane close: worker clears metadata, releases agent, pane re-eligible for detection.

## Extending

Drop `<agent_name>.yml` into `$HERDR_PLUGIN_CONFIG_DIR/frameworks/` — no code change, no registration. Loaded at startup, every `*.yml`/`*.yaml`.

| Field               | Type            | Notes                                                                                                       |
| ------------------- | --------------- | ----------------------------------------------------------------------------------------------------------- |
| `schema_version`    | int             | must match loader's supported version                                                                       |
| `source`            | string          | unique, e.g. `custom:vite`                                                                                  |
| `agent_name`        | string          | unique                                                                                                      |
| `display_agent`     | string          | sidebar label                                                                                               |
| `starting_message`  | string          | shown on first confirm                                                                                      |
| `state_labels`      | map             | `idle`/`working`/`blocked`, all required                                                                    |
| `clear_token_names` | list            | optional                                                                                                    |
| `bin_path_pattern`  | regex           | vs resolved argv; must not match `""`                                                                       |
| `package_name`      | string          | expected `package.json` `name`                                                                              |
| `signals`           | list            | `kind` (`starting`/`ready`/`error`/`building`/`recovered`) + `needle` + optional `counts_as_recent_success` |
| `url_pattern`       | regex, optional | exactly 2 capture groups: URL, port                                                                         |

### Schema Validation

Run command:

```bash
herdr plugin action invoke herdr-devserver-status.validate-report-popup
```

## License

MIT — see [LICENSE](LICENSE).

## References

- [Herdr documentation](https://herdr.dev/docs/plugins/)
