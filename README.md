# nodenuke 💣

Find and delete `node_modules` directories across your entire machine — fast,
safe, and without leaving your terminal.

**Inspired by [npkill](https://github.com/voidcosmos/npkill) but rewritten from scratch in Rust, with a focus on speed, safety, and a polished user experience.**

**Personal note:** 
>I originally wrote this tool as a simple script. However, when I decided to add a terminal UI and improve code readability, it turned into a complete nightmare and took much longer than expected. I ended up using AI to handle the refactoring, TUI implementation, and documentation. If you find anything strange or suboptimal, please feel free to open an issue or a PR to help improve it.

---

## Installation

### npx — no install required

The fastest way to try it. Works on any machine with Node.js installed.

```sh
npx nodenuke
npx nodenuke ~/projects
```

### npm — global install

```sh
npm install -g nodenuke
```

### Homebrew — macOS and Linux

```sh
brew tap shacript/nodenuke
brew install nodenuke
```

### cargo

If you have a Rust toolchain installed:

```sh
cargo install nodenuke
```

### Shell script — macOS and Linux

```sh
curl -fsSL https://raw.githubusercontent.com/shacript/nodenuke/main/install.sh | sh
```

### Pre-built binaries

Download the binary for your platform from the
[Releases](https://github.com/shacript/nodenuke/releases) page and put it
somewhere on your `PATH`.

| Platform | File |
|---|---|
| macOS — Apple Silicon | `nodenuke-aarch64-apple-darwin` |
| macOS — Intel | `nodenuke-x86_64-apple-darwin` |
| Linux — x64 | `nodenuke-x86_64-unknown-linux-musl` |
| Linux — arm64 | `nodenuke-aarch64-unknown-linux-musl` |
| Windows — x64 | `nodenuke-x86_64-pc-windows-msvc.exe` |

---

## Usage

```sh
nodenuke                # scan the current directory
nodenuke ~/projects     # scan a specific directory
nodenuke --help         # print usage
nodenuke --version      # print version
```

nodenuke scans the path you give it (or `.` if you don't give one), finds
every `node_modules` directory, and presents them in a list. From there you
pick what to delete and confirm. That's it.

---

## Sensitive paths

nodenuke automatically detects directories that look like they might be managed
by an application or the operating system. These entries are marked with a red
`⚠` in front of their path and are **not** pre-selected.

You can still select sensitive entries manually with `Space`, or include all of
them at once with `A`. When at least one sensitive entry is selected, the
confirmation popup shows a warning before anything is deleted.

The detection rules are intentionally conservative. It is better to flag
something as sensitive and let you handle it manually than to silently delete
something load-bearing.

**Locations that are always flagged:**

- `~/.config/**` — application configuration
- `~/.local/share/**` — XDG application data
- `~/.cache/**` — caches managed by other tools
- Any other hidden top-level directory under `~` (e.g. `~/.myapp`)
- `/Applications/Foo.app/**` — macOS application bundles
- `AppData\Roaming\**` — Windows roaming application data
- `AppData\Local\**` — Windows local application data (with exceptions below)
- UNC network paths containing hidden directory segments

**Locations that look sensitive but are safe to delete:**

- `~/.npm` and `~/.pnpm` — package manager download caches; deleting them
  just means the next install re-downloads packages
- `AppData\Local\.cache`, `AppData\Local\.npm`, `AppData\Local\.pnpm` —
  same reasoning on Windows

---

## Building from source

You need a [Rust toolchain](https://rustup.rs) (stable, 1.85 or newer).

```sh
git clone https://github.com/shacript/nodenuke
cd nodenuke
cargo build --release
./target/release/nodenuke
```

To install it to your Cargo bin directory:

```sh
cargo install --path .
```

---

## License

MIT
