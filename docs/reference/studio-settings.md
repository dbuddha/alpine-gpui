# Alpine Studio settings

Alpine Studio reads local JSON settings only. It does not load extensions,
plugins, executable configuration, remote schemas, accounts, AI services, or
telemetry.

## Files and precedence

1. Compiled defaults.
2. Global settings at
   `~/Library/Application Support/Alpine Studio/settings.json`.
3. Project settings at `<workspace>/.alpine/settings.json`.

Each later layer overrides the earlier layer. Missing files are ignored. A
malformed or unsupported layer rejects the complete reload and preserves the
previous active snapshot.

Files are capped at 64 KiB. Paths, JSON depth, parsed values, string bytes,
bindings, font names, and retained settings all have independent ceilings.
Reload work runs through the bounded background worker pool. Only the exact
current generation may publish.

## Version 1

```json
{
  "version": 1,
  "editor": {
    "font_name": "Menlo-Regular",
    "font_size": 15,
    "font_scale": 2,
    "line_height": 22,
    "tab_columns": 4
  },
  "theme": {
    "background": [0.035, 0.04, 0.045, 1.0],
    "caret": [0.94, 0.72, 0.25, 1.0],
    "syntax": {
      "comment": [0.48, 0.60, 0.53, 1.0]
    }
  },
  "keymap": {
    "bindings": [
      {
        "physical_key": 1,
        "modifiers": ["command"],
        "action": "save_file",
        "label": "Cmd+S"
      }
    ]
  }
}
```

Editor and theme objects are partial. A supplied keymap is a complete
replacement and can contain at most 64 bindings. Theme channels are linear
RGBA values in the inclusive range `0.0..=1.0`. The v1 schema accepts only the
compiled `Menlo-Regular` font name because dynamic font registration is not a
shipping boundary.

The command palette action `Preferences: Reload Settings` requests a coalesced
reload. It never blocks input or rendering on filesystem work.

## Version 0 migration

Version 0 accepts only top-level `font_size`, `font_scale`, `line_height`, and
`tab_columns`. Alpine migrates those values into the version 1 editor layer in
memory. Unknown versions fail closed; Studio never rewrites the source file.
