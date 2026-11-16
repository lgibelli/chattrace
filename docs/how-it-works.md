# How it works

## Where Teams keeps its cache

The new Microsoft Teams client (2023+) is a WebView2/WebKit app. It keeps chats, conversations
and profiles in a Chromium **IndexedDB**, which is stored as a **LevelDB** folder inside its web
profile:

| OS | Folder |
|---|---|
| macOS | `~/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView/<profile>/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb` |
| Windows | `%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\Microsoft\MSTeams\EBWebView\<profile>\IndexedDB\https_teams.microsoft.com_0.indexeddb.leveldb` |

`<profile>` is `WV2Profile_tfw` for work/school accounts and `WV2Profile_tfl` for personal
accounts (whose origin is `teams.live.com`). Large values are stored next to it in
`…indexeddb.blob`. Classic Teams (retired 2024–2025) used
`~/Library/Application Support/Microsoft/Teams/IndexedDB` or `%APPDATA%\Microsoft\Teams\IndexedDB`.
ChatTrace finds and preserves those caches too, but does not decode them.

Inside, Teams uses one set of databases per account, named
`Teams:<manager>:react-web-client:<tenant>:<user>:<locale>`:

| Database / store | Content |
|---|---|
| `replychain-manager` / `replychains`, `replychains-2` | messages, grouped in reply chains |
| `conversation-manager` / `conversations` | chats, meetings, channels, members, cache-purge history |
| `profiles` / `profiles` | people: MRI, name, e-mail, job title |

A change of display language creates a second set of databases, so the same message can be
stored twice. ChatTrace de-duplicates versions per account.

## Reading without writing

A LevelDB library cannot be used on evidence: opening a database takes its `LOCK` file, replays
the write-ahead log into a new table, and may compact, all of which write to the folder.
ChatTrace therefore reads the files itself, layer by layer:

| Layer | Module | What it does |
|---|---|---|
| LevelDB | `src/leveldb.rs` | reads `.log` files (32 KiB blocks, record fragments, write batches) and `.ldb` tables (footer, index, Snappy-compressed blocks, prefix-compressed keys), checking every CRC32C |
| IndexedDB | `src/idb.rs` | decodes Chromium's key prefixes (database / store / index IDs), database and store names, value envelopes, Snappy-wrapped values and values moved to blob files |
| V8 | `src/v8.rs` | decodes the V8 / Blink structured-clone format (objects, arrays, maps, sets, dates, strings, references, Blink host objects) into JSON |
| Teams | `src/teams.rs` | builds accounts, conversations, messages and people; converts message HTML to text; describes calls and meeting events; groups and compares versions |
| Discovery | `src/discover.rs` | finds Teams caches on this computer or under any folder |
| Evidence | `src/evidence.rs`, `src/report.rs` | copy + hash, CSV/HTML/JSON/text, manifest, zip, verification |

Every file is opened read-only. The evidence export parses the **copy** it has just made and
verified, not the original.

## Why earlier versions are recovered

LevelDB never edits data in place. A change to a record is written as a new entry with a higher
sequence number, and a deletion is written as a tombstone. Old entries stay in the files until
*compaction* merges tables and drops them, which can take a long time. A library shows only the
newest entry of each key. ChatTrace reads **all** of them and labels each one:

- `live`: the newest entry of its key;
- `superseded`: an older entry, overwritten later but still in the files;
- `deleted`: an entry whose key was later deleted.

Teams stores a whole reply chain (a thread with its messages) as one record, so an older entry of
a reply chain can contain an older state of a message: its text before an edit, before it was
deleted, or before a reaction was added. ChatTrace extracts every message from every entry,
fingerprints each distinct version, and compares older versions with the latest one. It reports
which fields differ (`changes_vs_latest`) and whether the difference is visible to a reader
(`substantive_change`).

## Gaps Teams records

The conversation records include a `cacheDeleteHistory`: each time Teams purged reply chains from
its local cache, it noted when, how many reply chains were removed, and which sync window they
belonged to. ChatTrace shows these purges for each conversation, so a gap in the history is
documented rather than silent.
