# Updating Existing Local Plugins

Use this reference when a plugin already exists and the request is about updating it during local development. Keep the original scaffold flow in `SKILL.md` for creating new plugins.

This flow stays CLI-driven:

- use Codex CLI to confirm the marketplace the plugin already uses
- update the plugin manifest version with a cachebuster suffix instead of incrementing numeric semver parts
- reinstall the updated plugin with `codex plugin add`
- start a new thread afterward so Codex picks up the updated plugin and tools

## When To Use This Flow

Use this flow when all of the following are true:

- the plugin already exists locally
- the marketplace entry already points at the plugin source you are editing
- the user wants Codex to see the updated plugin without manually editing marketplace files

If the user still needs the initial plugin entry or marketplace structure created, use the scaffold flow first and only then switch to this reinstall flow.

## CLI-Driven Loop

1. Update the plugin manifest to a single Codex cachebuster suffix:

```bash
python3 .agents/skills/plugin-creator/scripts/update_plugin_cachebuster.py <plugin-path>
```

Prefer the default helper behavior here. If you omit `--cachebuster`, the helper uses a UTC timestamp down to seconds, which is the recommended path for routine local iteration.

Only use a manual cachebuster override when the user explicitly asks for one or when a workflow outside Codex depends on a specific token:

```bash
python3 .agents/skills/plugin-creator/scripts/update_plugin_cachebuster.py <plugin-path> \
  --cachebuster local-20260519-184516
```

2. Check which local marketplace the plugin is installed from:

```bash
codex plugin marketplace list
```

The default scaffolded plugin flow uses the `personal` marketplace, so prefer reinstalling from `personal`.

If the plugin is not in `personal`, confirm which marketplace entry points at the plugin source you are editing and make sure that marketplace is still local. If it is a different local marketplace, reinstall from that marketplace name instead of forcing `personal`. If it is not local, stop and help the user resolve the mismatch before continuing.

3. Reinstall the updated plugin after changing its contents:

```bash
codex plugin add <plugin-name>@personal
```

If the plugin lives in a different confirmed local marketplace, substitute that marketplace name:

```bash
codex plugin add <plugin-name>@<local-marketplace>
```

4. Start a new thread so Codex picks up the updated plugin and tools.

## Cachebuster Policy

- Preserve the existing version prefix and replace only the suffix.
- Treat the preserved prefix as everything before `+`.
- Use the format:

```text
<base-version>+codex.<cachebuster>
```

Examples:

- `0.1.0` → `0.1.0+codex.local-20260519-184516`
- `0.1.0+codex.old-token` → `0.1.0+codex.local-20260519-184516`
- `1.2.3-beta.1+codex.prev` → `1.2.3-beta.1+codex.local-20260519-184516`
- `dev-build+other-tag` → `dev-build+codex.local-20260519-184516`

Replace the existing Codex cachebuster instead of appending another one. Do not keep incrementing numeric version components just to trigger reinstall behavior.

## Marketplace Rules

- Marketplace manipulation should happen through Codex CLI, not by hand-editing `marketplace.json` or `config.toml` during this update/reinstall flow.
- Use `codex plugin marketplace list` to confirm the plugin is being reinstalled from the expected marketplace.
- Prefer `personal` for the default scaffolded flow.
- If the plugin is not in `personal`, confirm that the selected marketplace is local before telling the user to reinstall from it.
- If the selected marketplace is not local, stop and help the user resolve that mismatch rather than pretending the normal local reinstall flow applies.
- If the plugin source is not already the source referenced by the chosen marketplace entry, stop and fix that first. This update flow does not rewrite marketplace entries.

## After Reinstall

After reinstalling, start a new thread. That is the safe boundary for picking up the updated plugin and its MCP tools.
