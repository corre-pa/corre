# corre-cli

The binary entry point for the Corre project. Compiles to the `corre` executable and wires
every other crate in the workspace together into a working program.

## Role in the Corre project

`corre-cli` sits at the top of the dependency graph. It imports all other workspace crates and
composes them into four runtime modes. No other crate depends on `corre-cli`.

## CLI commands

```
corre [OPTIONS] <COMMAND>

Options:
  -c, --config <CONFIG>  Path to corre.toml

Commands:
  run           Start the full daemon (scheduler + web server)
  run-now       Run a single app immediately and exit
  serve         Start only the CorreNews web server
  setup         Launch the interactive first-run wizard
  install-deps  Check and install required external dependencies
```

### `corre run`

Starts both the cron scheduler and the web server. Apps run in isolated tokio tasks
with a 10-minute timeout and progress polling.

### `corre run-now <app>`

Runs a single app synchronously using the same pipeline as `run`.

### `corre serve`

Starts only the web server and dashboard (no scheduler).

### `corre setup`

Nine-step interactive wizard: dependency checks, LLM provider, Brave Search, app
selection, per-app LLM overrides, topics, preferences, config writing, and start options.
State persists to `{data_dir}/.setup-state.json` for resumption.

## Module layout

```
src/
  main.rs              CLI parsing, command dispatch, app pipeline
  setup/
    mod.rs             Wizard entry point and step sequencing
    deps.rs            Dependency detection and auto-installation
    providers.rs       LLM provider catalog
    state.rs           Serializable wizard state
    steps.rs           Individual wizard steps (1-9)
    systemd.rs         systemd unit file generation (Linux)
    templates.rs       Config file rendering and data-dir resolution
    validate.rs        Input validators for dialoguer prompts
```
