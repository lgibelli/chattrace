# The evidence package

One export produces two files:

- `<name>.zip`: the package;
- `<name>.zip.sha256`: the SHA-256 of the zip, in `sha256sum` format.

## How it is made

1. Every file of the Teams IndexedDB folder (`…indexeddb.leveldb`) and its blob folder
   (`…indexeddb.blob`) is opened **read-only** and copied byte for byte. SHA-256 is computed
   over the bytes as they are read. The copy is then hashed again and must match. Size and
   modification time are compared before and after each file, and any file that changed during
   the copy is flagged. Modification times are preserved on the copy.
2. The readable files are generated **from the copy**, never from the live files, by
   ChatTrace's own parser.
3. `manifest.json`, `README.txt` and `SHA256SUMS` are written. The zip is built, reopened, and
   every entry is checked against `SHA256SUMS`. Then the zip's own SHA-256 is written next to it.

The source is never written to. No database library opens it (opening a LevelDB with a library
takes its lock and may rewrite files).

## Contents

```
README.txt          what is what, and how to verify
manifest.json       capture details, coverage, warnings, limitations, SHA-256 of every file
SHA256SUMS          sha256sum-compatible list of every file (except itself)
original/
  https_teams.microsoft.com_0.indexeddb.leveldb/   byte-for-byte copy (.log, .ldb, MANIFEST, CURRENT, LOG…)
  https_teams.microsoft.com_0.indexeddb.blob/      byte-for-byte copy, if present
readable/
  report/index.html            HTML report: capture details, Purview note, accounts, conversations
  report/conversations/*.html  one page per conversation, every message with all metadata + raw record
  messages.csv                 one row per message version
  conversations.csv            conversations, members, cache purges
  participants.csv             people known to the cache
  transcripts/<account>/*.txt  plain-text transcript per conversation
  messages.jsonl               full decoded messages with provenance
  conversations.jsonl, accounts.jsonl
  all_records.jsonl            every record of every IndexedDB store, every version (unless --chats-only)
  stores.csv                   record counts per store and status (with all_records)
  leveldb_files.jsonl          per database file: records, CRC errors, parse errors
  decode_errors.jsonl          chat records that could not be decoded
```

CSV files are UTF-8 with a byte-order mark, so Excel opens them correctly. Times named `*_utc`
are RFC 3339 in UTC with milliseconds. `*_local` adds the capture computer's time zone, which is
stored in the manifest.

## The HTML report

Open `readable/report/index.html` in any browser. It works offline and loads nothing from the
network. Each page carries a Content-Security-Policy that blocks scripts and remote content, and
message text is always shown as escaped text, never interpreted as HTML. So opening the report
cannot trigger tracking images or run anything embedded in a chat.

The first page has the capture details (case, examiner, notes, times, host, OS, Teams version,
source path, read-only status of the source volume, whether Teams was running), the warnings, a
table of accounts and conversations, the limitations, and how to verify. Each conversation page
shows the conversation record and members, any recorded cache purges, and then every message.
For each message it shows local and UTC time, sender and MRI, every ID, edit and delete
information, reactions with who and when, mentions, files, version status, the source record and
the full raw record. Earlier versions are shown indented after the current version. The pages
print well.

## messages.csv

One row for each distinct version of each message. The current version has `variant = latest`;
older versions recovered from the database have `variant = earlier`.

| Column | Meaning |
|---|---|
| `account` | account key (see `accounts.jsonl`) |
| `conversation_id`, `conversation_title` | Teams conversation ID (`19:…`) and its title |
| `message_id` | Teams message ID (also its arrival timestamp in ms) |
| `version` | Teams version stamp of this state of the message |
| `variant` | `latest` or `earlier` |
| `versions_known` | number of distinct versions of this message in the cache |
| `changes_vs_latest` | for earlier versions: fields that differ from the latest (`content`, `properties.edittime`, …) |
| `substantive_change` | `yes` when the difference is visible to a reader (text, edit/delete markers, reactions, files, mentions) |
| `arrival_utc`, `arrival_local` | server arrival time (`originalArrivalTime`) |
| `client_arrival_utc` | when the client received it |
| `compose_time` | compose time, if recorded |
| `sender_mri`, `sender_name` | sender's Teams ID (MRI) and display name; `System` for thread events |
| `sent_by_account_owner` | `yes` when sent by the account whose cache this is |
| `message_type`, `content_type` | e.g. `RichText/Html`, `Text`, `Event/Call`, `ThreadActivity/AddMember` |
| `text` | readable text (HTML converted; calls, meeting events and member changes described) |
| `content_raw` | content exactly as stored (usually HTML) |
| `edited_utc`, `deleted_utc`, `deleted_by` | edit and delete markers recorded by Teams |
| `reactions` | `emoji: name (time)` for every reaction |
| `mentions` | mentioned people |
| `files` | `name <url>` of attached files |
| `importance`, `subject` | message importance and subject, if set |
| `reply_chain_id`, `client_message_id`, `sequence_id` | threading and ordering IDs |
| `properties_json` | the complete `properties` object as JSON |
| `source_database`, `source_store`, `source_file` | IndexedDB database, object store and LevelDB file the record came from |
| `leveldb_seq` | LevelDB sequence number of the record |
| `record_status` | `live` (newest version of the key), `superseded` (overwritten, not yet compacted) or `deleted` (key deleted) |
| `crc_ok` | whether the record's block passed its CRC check |
| `occurrences` | number of database records holding exactly this version |
| `version_fingerprint` | hash identifying this version (used to de-duplicate) |

## conversations.csv and participants.csv

`conversations.csv`: `account`, `conversation_id`, `type` (group chat, 1:1 chat, meeting,
channel…), `title`, `created_utc`, `last_activity_utc`, `messages_in_cache`, `members` (`name
[role] MRI; …`), `cache_purges_recorded` (JSON list of purges Teams recorded: when, how many
reply chains, which sync window), `has_metadata` (`no` when known only from its messages), and
the source columns.

`participants.csv`: `mri`, `display_name`, `email`, `user_principal_name`, `job_title`,
`tenant_name`, `source_database`.

## manifest.json

```jsonc
{
  "tool":    { "name": "ChatTrace Forensic", "version": "…" },
  "capture": {
    "case_reference": "…", "examiner": "…", "notes": "…",
    "started_utc": "…", "started_local": "…", "finished_utc": "…", "timezone": "Europe/Rome",
    "host": { "hostname": "…", "os": "macos", "os_version": "…", "os_user": "…", "teams_app_version": "…" },
    "source_paths": ["…indexeddb.leveldb", "…indexeddb.blob"],
    "source_kind": "New Teams · work/school",
    "source_is_live_local_profile": true,
    "source_volume_read_only": false,
    "teams_running_during_capture": false,
    "method": "…", "warnings": [], "limitations": ["…"]
  },
  "coverage": { "accounts": [ … per-account counts and first/last message … ], "conversations": 0, "messages": 0,
                "earlier_versions": 0, "earlier_versions_substantive": 0, "first_message_utc": "…", "last_message_utc": "…" },
  "parse":    { "leveldb_files": 0, "crc_errors": 0, "chat_decode_errors": 0 },
  "files":    [ { "path": "original/…", "size": 0, "sha256": "…", "modified_utc": "…", "created_utc": "…",
                  "source_path": "…", "changed_during_copy": false }, … ]
}
```

`host` describes the computer that ran ChatTrace. When the source is an external disk or an
image, `source_is_live_local_profile` is `false`, and `teams_app_version` refers to the examiner's
computer, not the source.

## Verifying

With ChatTrace: `chattrace-cli verify <zip>` or **Verify…** in the app. With standard tools:

```sh
shasum -a 256 -c case.zip.sha256                  # the zip as a whole (Linux: sha256sum -c)
unzip case.zip -d case && cd case
shasum -a 256 -c SHA256SUMS                       # every file inside
```

On Windows PowerShell: `Get-FileHash case.zip -Algorithm SHA256`, then compare with the `.sha256` file.

## Trusted timestamp (recommended)

A hash only shows that the package has not changed since some unknown moment. An RFC 3161
timestamp from a timestamping authority proves the package existed at a given time. Only
the hash leaves your computer:

```sh
openssl ts -query -data case.zip -sha256 -cert -out case.tsq
curl -s -H 'Content-Type: application/timestamp-query' --data-binary @case.tsq https://freetsa.org/tsr -o case.tsr
openssl ts -reply -in case.tsr -text                 # inspect
# verify later (download the TSA's cacert.pem and tsa.crt from freetsa.org):
openssl ts -verify -data case.zip -in case.tsr -CAfile cacert.pem -untrusted tsa.crt
```

Keep `case.tsr` with the zip. Keep at least two copies of the package on different media, and
work only on copies.

## Limitations

The same limitations are stated in the package itself (`manifest.json`, `README.txt` and the
report):

- **Use Microsoft Purview whenever possible.** Its export is the authoritative, complete record.
  This package reflects only what one Teams client had cached on one computer.
- History the client never loaded, or that Teams purged from its cache, is absent. Purges that
  Teams recorded are listed.
- Earlier versions exist only until database compaction removes them. Their absence does not mean
  a message was never edited.
- Attachments are references (SharePoint/OneDrive links). File contents and most inline images
  are not part of the cache.
- Hashes show the files have not changed since capture. They do not by themselves prove the
  authenticity or completeness of the original conversation.
