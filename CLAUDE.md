# voxloop

Software-based voice loop system — see `inital-ramble.md` for the originating brief.

## Agent skills

### Issue tracker

Issues live as GitHub issues on `edwardhutchinson/voxloop`, managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles, used verbatim as label strings. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Running it

`scripts/dev` builds the console, starts the server and makes an administrator, printing the
URL and password. See the README.

### Rules

Before generating any final prose for us conversing, you MUST invoke the `unslop` skill.
