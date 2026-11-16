# ChatTrace Forensic command line

The command-line tool has the same engine as the app and writes the same evidence packages.

| Platform | Command |
|---|---|
| Windows | `chattrace-cli.exe` (next to `chattrace.exe`). Use this one in a terminal: `chattrace.exe` is a windowed app and prints nothing. |
| Linux | `chattrace-cli` (or `chattrace <command>`) |
| macOS | `"/Applications/ChatTrace Forensic.app/Contents/MacOS/chattrace-cli"`. Tip: `ln -s "/Applications/ChatTrace Forensic.app/Contents/MacOS/chattrace-cli" /usr/local/bin/chattrace-cli` |

`chattrace` with no arguments opens the app. With a command it behaves like `chattrace-cli`.
`chattrace-cli` with no command prints help.

```
chattrace-cli <COMMAND> [OPTIONS]

  view       Open the viewer window, optionally on a path
  discover   List Teams caches on this computer, or anywhere under a path
  summary    Summary of the accounts, conversations and messages in a cache
  export     Create an evidence package (zip)
  verify     Verify an evidence zip
  update     Check for a newer version and install it
  license    Print the license and third-party notices
  stores, sample, fields   Advanced: inspect the raw IndexedDB stores

Global options:
  --no-update-check   Don't check GitHub for a newer version at startup (app)
  -h, --help          Help (also for each command: chattrace-cli export --help)
  -V, --version
```

Exit codes: `0` success, `1` error, `2` invalid arguments, `3` verification failed.

## Choosing the source

Commands that read a cache take `--source <path>` (short `-s`). The path can be anything:

- the `…indexeddb.leveldb` folder itself;
- a Teams profile, a user's home folder, or a copy of either;
- the root of an external disk or a mounted disk image (`/Volumes/Evidence`, `E:\`,
  `/mnt/image`).

ChatTrace looks for the known macOS and Windows Teams locations under the path, and under its
`Users/*` and `home/*` folders. If they are missing, it searches the tree (up to 16 levels,
skipping system folders). If exactly one cache is found it is used. If several are found,
the command stops and lists them, so you can pass the one you mean.

Without `--source`, the current user's own Teams cache is used. If there are several (work and
personal), the first is used and the others are listed.

## discover

```sh
chattrace-cli discover                 # this computer's user
chattrace-cli discover /Volumes/Disk   # any folder, disk or image
chattrace-cli discover E:\ --json      # machine-readable
```

For each cache it prints the type (*New Teams · work/school*, *New Teams · personal*, *Classic
Teams*), the full path, its size, file count, last change, the layout (macOS/Windows), the user
folder name, and whether it is this computer's live Teams profile.

## summary

```sh
chattrace-cli summary -s /Volumes/Disk/Users/jdoe
```

Prints, per account: the number of conversations, messages, earlier versions, edited and deleted
messages, and the first and last message time. Then it lists the earlier versions grouped by
what differs from the latest version (`*` marks differences a reader would notice), followed by
the number of people, decode errors, LevelDB files and CRC errors.

## export

```sh
chattrace-cli export [-s <path>] [-o <file.zip>] [--case <ref>] [--examiner <name>] [--notes <text>]
                     [--chats-only] [--keep-unzipped]
```

| Option | Meaning |
|---|---|
| `-s, --source <path>` | what to capture (see above) |
| `-o, --out <file.zip>` | zip to create; default `chattrace-<UTC time>.zip` in the current folder. Never overwritten. |
| `--case <ref>` | case or matter reference |
| `--examiner <name>` | person performing the capture |
| `--notes <text>` | free text: reason, scope, circumstances |
| `--chats-only` | leave out `all_records.jsonl` (every record of every IndexedDB store) |
| `--keep-unzipped` | also keep the unzipped package folder (`<name>.staging`) next to the zip |

```sh
chattrace-cli export --case 2026-041 --examiner "J. Doe" \
    --notes "Own chats, preserved before leaving the company" -o ~/Desktop/teams-2026-041.zip
```

Progress goes to stderr. When it finishes, it prints the zip path, the number of files and
messages, and the zip's SHA-256. It also writes `<zip>.sha256` next to the zip. Warnings (for
example *Teams was running*, or *a file changed while it was copied*) are printed and stored in
the manifest.

When capturing this computer's Teams, quit Teams first. The command warns if it is running.

## verify

```sh
chattrace-cli verify teams-2026-041.zip
```

Recomputes the zip's SHA-256 and checks it against the `.sha256` file next to it (if
present). It then recomputes every file inside and checks it against `SHA256SUMS`. It prints
`OK: the package is intact.`, or lists `MODIFIED`, `MISSING` and `UNLISTED` files and exits with
code 3.

The same check works with standard tools, without ChatTrace:

```sh
shasum -a 256 -c teams-2026-041.zip.sha256      # macOS / Linux (sha256sum -c on Linux)
unzip teams-2026-041.zip -d pkg && cd pkg && shasum -a 256 -c SHA256SUMS
```

## update

```sh
chattrace-cli update --check   # only report
chattrace-cli update           # download, verify signature + checksum, install
```

See [Updates](gui.md#updates) for how an update is verified.

## Advanced: stores, sample, fields

For examining the raw IndexedDB content, for example to study a new Teams version:

```sh
chattrace-cli stores                                   # databases, object stores, record counts per status
chattrace-cli sample --db replychain --store replychains -n 2 --trunc 80
chattrace-cli sample --db profiles --store profiles --contains "@example.com" --all
chattrace-cli fields --db conversation-manager --store conversations --show threadProperties
```

`--all` includes superseded and deleted record versions. `--db` matches part of the database
name.

## Environment

| Variable | Effect |
|---|---|
| `CHATTRACE_NO_UPDATE_CHECK=1` | never check for updates at startup |
