# zj-context-keys

[![CI](https://github.com/Townk/zj-context-keys/actions/workflows/ci.yml/badge.svg)](https://github.com/Townk/zj-context-keys/actions/workflows/ci.yml)
[![Latest build](https://img.shields.io/github/v/release/Townk/zj-context-keys?include_prereleases&label=latest)](https://github.com/Townk/zj-context-keys/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A tiny [Zellij](https://zellij.dev) plugin that lets you bind a key the way
you normally would in your config, but choose what it does based on the
**foreground process running in the focused pane**.

Two motivating examples:

- `Shift+Enter` should send `Shift+Enter` to the underlying application —
  **except** when that application is Cursor's `cursor-agent` CLI, in which
  case it should send `Alt+Enter`.
- `Ctrl+j` should run Zellij's `MoveFocus "Down"` — **except** when the
  focused pane is running `fzf`, in which case the raw `Ctrl+j` byte should be
  forwarded so `fzf` can move its selection.

The plugin runs headless: you load it once at session start and dispatch into
it from keybindings via `MessagePlugin`. Each keybinding carries its own rule
in the `payload`, so the logic lives next to the binding instead of in some
separate config blob.

## Install

Every release ships a prebuilt `zj-context-keys.wasm` — you don't need a Rust
toolchain to use the plugin. Two stable download URLs are published:

| URL | Tracks |
|---|---|
| `https://github.com/Townk/zj-context-keys/releases/download/latest/zj-context-keys.wasm` | The **rolling build** — refreshed on every push to `master`. |
| `https://github.com/Townk/zj-context-keys/releases/download/v0.1.2/zj-context-keys.wasm` | A **pinned version** — immutable once published. |

### Option A — reference the release URL directly (recommended)

Zellij can load a plugin straight from a URL and caches it locally, so you can
point your config at the release asset and skip the manual download entirely.
Use the URL anywhere this README writes `file:~/.config/zellij/plugins/zj-context-keys.wasm`:

```kdl
load_plugins {
    "https://github.com/Townk/zj-context-keys/releases/download/latest/zj-context-keys.wasm"
}
```

> Pin to a `v*` URL for reproducible setups; use the `latest` URL to always
> ride the newest build. After changing the URL, clear Zellij's plugin cache
> (`~/.cache/zellij`) or restart the session to force a re-download.

### Option B — download the asset into your plugins directory

```sh
mkdir -p ~/.config/zellij/plugins
curl -fL -o ~/.config/zellij/plugins/zj-context-keys.wasm \
  https://github.com/Townk/zj-context-keys/releases/download/latest/zj-context-keys.wasm
```

Then refer to it as `file:~/.config/zellij/plugins/zj-context-keys.wasm` (the
form used throughout the examples below).

## Build from source

If you'd rather build it yourself:

```sh
rustup target add wasm32-wasip1   # one-time
cargo build --release --target wasm32-wasip1
```

The artifact is `target/wasm32-wasip1/release/zj-context-keys.wasm`. Copy it to
wherever you keep Zellij plugins, e.g.:

```sh
mkdir -p ~/.config/zellij/plugins
cp target/wasm32-wasip1/release/zj-context-keys.wasm ~/.config/zellij/plugins/
```

`just install` does the build-and-copy in one step.

## Configure

In your `~/.config/zellij/config.kdl`:

```kdl
// Load the plugin once at session start so it's ready before any keybind fires.
load_plugins {
    "file:~/.config/zellij/plugins/zj-context-keys.wasm"
}

keybinds {
    shared {
        // Shift+Enter: pass through, but use Alt+Enter inside cursor-agent.
        // `$source` forwards the binding's own key (read from `name`).
        bind "Shift Enter" {
            MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
                name "shift+enter"
                payload "default: $source; when cursor-agent,agent: key alt+enter"
            }
        }

        // Ctrl+j: MoveFocus Down, but forward the raw Ctrl+j to fzf.
        bind "Ctrl j" {
            MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
                name "ctrl+j"
                payload "default: do move_focus down; when fzf: $source"
            }
        }
    }
}
```

> The `name` field doubles as the key spec for `$source`, so set it to the
> same key you bound (`"ctrl+j"`, `"shift+enter"`, …) using the notation
> below. If you never use `$source`, `name` can be any label you like.

Reload Zellij (or detach/reattach the session) and the bindings are live.

## Rule DSL

A `payload` is a sequence of clauses separated by `;` or newlines. Each clause
is either:

```
default: <action>
when <proc1>[,<proc2>...]: <action>
```

Where `<action>` is one of:

| Action | Description |
|---|---|
| `key <keyspec>` | Send a keystroke described in friendly notation (e.g. `ctrl+j`, `alt+enter`, `f5`). `send` is an alias. See "Key notation" below. |
| `$source` | Forward the binding's **own** key — send whatever the key in this `MessagePlugin`'s `name` field would normally produce. See "Forwarding the original key" below. |
| `write <bytes>` | Send raw bytes to the focused pane. The escape hatch when you need exact control. Supports `\r \n \t \0 \a \b \f \v \e \\ \" \'`, hex (`\xNN`), and Unicode (`\u{NNNN}`). |
| `do <name> [args…]` | Run a built-in Zellij command (see below). |
| `pipe <url> [name=…] [payload=…] [<cfg-key>=<cfg-value>]*` | Re-dispatch the keystroke into another plugin (equivalent to a Zellij `MessagePlugin` block). See "Re-piping" below. |
| `run <command> [args…]` | Execute an external command headlessly (no pane, no flash). See "Running external commands" below. |
| `noop` | Swallow the key. Also accepts `nothing` / `swallow`. |

Process names are matched case-insensitively against the **basename** of the
foreground command in the focused pane (e.g. `/usr/local/bin/cursor-agent`
matches `cursor-agent`) **and** against every descendant process of that
pane. The latter is what makes `when fzf` fire even when `fzf` was launched
by a zsh widget, a shell function, or `$(…)` substitution — situations
where the pty's foreground process group keeps reporting the shell.
Multiple names can be combined with commas. The first matching `when` wins;
otherwise the `default` runs.

The descendant list is refreshed every 500 ms via `ps -A` (and again
immediately after each keystroke), so detection latency is well below
one second.

Lines beginning with `#` are comments.

### Built-in `do` commands

| Name | Args | Notes |
|---|---|---|
| `move_focus` | `up`/`down`/`left`/`right` | |
| `move_focus_or_tab` | direction | Wraps across tab boundaries. |
| `move_pane` | optional direction | No arg ⇒ enters move mode. |
| `new_tab` | optional name | |
| `go_to_next_tab` / `go_to_previous_tab` | — | |
| `go_to_tab` | tab index (u32) | |
| `go_to_tab_name` | tab name | |
| `close_focus` / `close_pane` | — | Closes the focused pane. |
| `close_tab` | — | Closes the focused tab. |
| `toggle_pane_embed_or_eject` | — | Floats/embeds the pane. Alias: `toggle_floating`. |
| `toggle_pane_frames` | — | |
| `toggle_fullscreen` | — | |
| `switch_mode` | mode name | e.g. `Normal`, `Locked`, `Resize`, … |
| `detach` | — | |
| `quit` | — | |
| `write_chars` | string | Like `write` but UTF-8 text without escapes. |

Hyphenated and snake_case spellings are both accepted (`move-focus` ≡ `move_focus`).

### Key notation

`key <keyspec>` (alias `send`) is the friendly alternative to `write`: you
describe the keystroke instead of spelling out its bytes. The spec is decoded
to bytes at parse time, so `key ctrl+j` and `write \x0a` are identical once
loaded.

Grammar — zero or more modifiers, then a base key, joined with `+` (or `-`),
case-insensitively:

```
[<mod>+]* <base>
```

| Modifiers | Aliases |
|---|---|
| `ctrl` | `control` |
| `alt` | `opt`, `option`, `meta` |
| `shift` | — |
| `super` | `cmd`, `command`, `win`, `hyper` |

Base keys are either a single character (`a`, `/`, `5`, …) or a named key:
`enter` (`return`), `tab`, `esc` (`escape`), `space`, `backspace`, `delete`,
`insert`, `up`/`down`/`left`/`right`, `home`, `end`, `pageup`/`pagedown`, and
`f1`–`f12`.

Encoding follows the **legacy** terminal conventions:

| Spec | Bytes | Notes |
|---|---|---|
| `key ctrl+j` | `\x0a` | `ctrl+<letter>` → control byte |
| `key ctrl+c` | `\x03` | |
| `key alt+enter` | `\x1b\r` | `alt+X` prepends `ESC` to `X` |
| `key shift+tab` | `\x1b[Z` | backtab |
| `key up` | `\x1b[A` | |
| `key ctrl+up` | `\x1b[1;5A` | modifiers fold into the xterm `;<code>` form |
| `key f5` | `\x1b[15~` | |
| `key esc` | `\x1b` | |

**The `shift+enter` caveat.** A few keys have no distinct *legacy* encoding —
`shift+enter` is byte-for-byte identical to plain `enter` (`\r`) unless the
focused application speaks the [kitty keyboard
protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/). For those,
prefix the spec with `kitty:` to emit the CSI-u form:

```
key kitty:shift+enter   # → \x1b[13;2u
```

The plugin only stuffs bytes into the pane; it can't change how your terminal
would have encoded the key, so use `kitty:` only with apps that negotiate that
protocol.

### Forwarding the original key

`$source` (bare `source` also works) sends whatever the binding's **own** key
would normally produce. It reads the key from this `MessagePlugin`'s `name`
field and runs it through the same notation as `key`, so:

```kdl
bind "Ctrl j" {
    MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
        name "ctrl+j"
        payload "default: do move_focus down; when fzf: $source"
    }
}
```

Here `when fzf: $source` forwards a real `Ctrl+j` (`\x0a`) to fzf without you
restating the bytes. If `name` isn't a valid key spec, `$source` logs an error
and does nothing (other clauses are unaffected).

To forward the key in its **kitty** (CSI-u) form, write `kitty:$source` — it
behaves like `$source` but forces the kitty encoding regardless of `name`:

```kdl
bind "Shift Enter" {
    MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
        name "shift+enter"
        // → \x1b[13;2u to apps that speak the kitty protocol.
        payload "default: kitty:$source; when cursor-agent: key alt+enter"
    }
}
```

(Equivalently, you can bake the prefix into `name` as `"kitty:shift+enter"`
and use a plain `$source`; `$source` honors a `kitty:` prefix already on
`name`.) Only send the kitty form to applications that negotiate the
protocol — others will see the literal escape sequence.

### Running external commands

The `run` action executes a command through Zellij's host `run_command` API.
Because the plugin is headless, **nothing is rendered** — there's no
transient pane and therefore no visual flash, unlike a Zellij `Run {}`
keybinding block.

Grammar:

```
run <command> [<arg>...]
```

- `<command>` is the program; remaining tokens are its arguments. Tokens are
  whitespace-delimited (no quoting in this revision), so use an absolute path
  and a wrapper script if you need spaces.
- Two placeholder tokens are substituted at execution time against the
  **focused terminal pane**:
  - `{pid}` → that pane's root process PID.
  - `{pane_id}` → its numeric terminal pane id.
- `{pid}` is the escape hatch for "what is the focused pane's cwd?": the host
  does **not** guarantee `ZELLIJ_*` env vars for `run_command` children, so a
  helper can't reliably call `zellij action …`. Instead it derives state from
  the PID, e.g. `cwd=$(lsof -a -p {pid} -d cwd -Fn | sed -n 's/^n//p')` on
  macOS. If `{pid}` is requested but can't be resolved, the command is **not**
  run (rather than firing with a bogus value).
- The command's stdout/exit status are ignored.

Example — copy the focused pane's working directory to the clipboard with a
notification, with zero flash:

```kdl
bind "Y" {
    MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
        name "copy-pwd"
        payload "default: run /Users/me/.config/zellij/scripts/copy-pwd {pid}"
    }
    SwitchToMode "normal"
}
```

### Re-piping into another plugin

The `pipe` action lets a single keybinding chain through *multiple* plugins
without bloating your Zellij config. Use it when you want this plugin's
context-aware routing on top of an existing plugin (e.g.
[`vim-zellij-navigator`](https://github.com/hiasr/vim-zellij-navigator)).

Grammar (single line, all on one rule clause):

```
pipe <plugin-url-or-alias> [name=<msg-name>] [payload=<msg-payload>] [<cfg-key>=<cfg-value>]*
```

- The URL is taken verbatim as the first whitespace-delimited token; both
  `file:…`, `https://…`, and Zellij plugin aliases work.
- `name=…` and `payload=…` populate the target's `PipeMessage`.
- Every other `key=value` flows into the target plugin's `load()`
  configuration map (the same one a normal `MessagePlugin` block populates
  from its child nodes).
- Values may not contain spaces in this revision (use a downstream plugin if
  you need that). They *may* contain `=`; only the first `=` per token is
  treated as the separator.

Equivalent of:

```kdl
MessagePlugin "https://…/vim-zellij-navigator.wasm" {
    name "move_focus"
    payload "down"
    move_mod "ctrl"
    use_arrow_keys "false"
}
```

written as a `pipe` clause:

```
pipe https://…/vim-zellij-navigator.wasm name=move_focus payload=down move_mod=ctrl use_arrow_keys=false
```

Combined with a `when`:

```kdl
bind "Ctrl j" {
    MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
        payload "default: pipe https://…/vim-zellij-navigator.wasm name=move_focus payload=down move_mod=ctrl use_arrow_keys=false; when fzf: write \\n"
    }
}
```

Requires the `MessageAndLaunchOtherPlugins` permission (the plugin requests
it on load).

## More examples

```kdl
// Esc: pass through, but in cursor-agent send Ctrl+C instead.
bind "Esc" {
    MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
        name "esc"
        payload "default: $source; when cursor-agent: key ctrl+c"
    }
}

// Ctrl+l: clear screen by default, swallow inside an SSH session.
bind "Ctrl l" {
    MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
        name "ctrl+l"
        payload "default: $source; when ssh: noop"
    }
}

// Alt+1..Alt+3: jump tabs, but let vim handle them when vim is focused.
bind "Alt 1" {
    MessagePlugin "file:~/.config/zellij/plugins/zj-context-keys.wasm" {
        name "alt+1"
        payload "default: do go_to_tab 1; when vim,nvim: $source"
    }
}
```

## How it works

1. The plugin loads in the background (no visible pane, never selectable).
2. It requests `ReadApplicationState`, `ChangeApplicationState`,
   `WriteToStdin`, `RunCommands`, and `MessageAndLaunchOtherPlugins`
   permissions.
3. A 500 ms timer polls `ps -A -o pid=,ppid=,comm=` and walks the focused
   pane's process subtree (rooted at `get_pane_pid`) to maintain a set of
   descendant process basenames.
4. When a keybinding fires `MessagePlugin`, Zellij delivers a `PipeMessage`.
5. The plugin parses the `payload`, takes the union of the pane's immediate
   foreground command (`get_pane_running_command`) and the descendant set
   as the candidate list, picks the first matching action, and executes it
   — either `write()`ing raw bytes to the pane or invoking the requested
   Zellij command.

## Limitations

- Process matching is by **basename** only. If you need to disambiguate
  (e.g. `node bin/script.js`), pick a wrapper name or rename the launcher.
- Descendant detection has up to ~500 ms of latency: a key pressed within
  the first half-second of a process spawning may still see the previous
  state. In practice fzf shows up well before you can react to it, so this
  is rarely visible.
- The plugin only sees the focused **terminal** pane. Floating plugin panes
  and other non-terminal targets are ignored (no `when` will match).
- Some keys (e.g. `Shift+Enter`) only carry information beyond plain Enter if
  your terminal speaks the kitty keyboard protocol. The plugin can only send
  whatever bytes you tell it to send; use `key kitty:…` to emit the CSI-u
  form for apps that negotiate that protocol.
- No globbing on process names; commas only. PRs welcome.

## Development

The host-agnostic logic (rule DSL, key-spec encoding, process-tree walking)
lives in the library crate and is unit-tested on the host toolchain — no WASM
runtime required:

```sh
cargo test                 # run the unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

A [`justfile`](justfile) wraps the common tasks (`just build`, `just test`,
`just lint`, `just install`). CI runs the same lint/test/build matrix on every
push, and pushing a `vX.Y.Z` tag publishes a versioned release.

## License

Licensed under the [MIT License](LICENSE).
