// SPDX-License-Identifier: GPL-3.0-or-later
//! Chromium IndexedDB-on-LevelDB decoding (content/browser/indexed_db/
//! indexed_db_leveldb_coding.cc and blink idb_value_wrapping.cc).

use crate::leveldb::{self, read_varint};
use crate::v8::{self, JsValue};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Prefix {
    pub db: u64,
    pub store: u64,
    pub index: u64,
}

const OBJECT_STORE_DATA: u64 = 1;
const BLOB_ENTRY: u64 = 3;

fn read_le_int(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |acc, &x| (acc << 8) | x as u64)
}

pub fn decode_prefix(key: &[u8]) -> Option<(Prefix, &[u8])> {
    let first = *key.first()?;
    let db_len = ((first >> 5) & 0x7) as usize + 1;
    let os_len = ((first >> 2) & 0x7) as usize + 1;
    let ix_len = (first & 0x3) as usize + 1;
    let total = 1 + db_len + os_len + ix_len;
    if key.len() < total {
        return None;
    }
    let mut p = 1;
    let db = read_le_int(&key[p..p + db_len]);
    p += db_len;
    let store = read_le_int(&key[p..p + os_len]);
    p += os_len;
    let index = read_le_int(&key[p..p + ix_len]);
    p += ix_len;
    Some((Prefix { db, store, index }, &key[p..]))
}

fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn string_with_length(b: &[u8], pos: &mut usize) -> Result<String> {
    let n = read_varint(b, pos)? as usize;
    let end = *pos + n * 2;
    if end > b.len() {
        bail!("truncated string");
    }
    let s = utf16be(&b[*pos..end]);
    *pos = end;
    Ok(s)
}

/// An IndexedDB key (the user-visible primary key of a record).
#[derive(Debug, Clone, PartialEq)]
pub enum IdbKey {
    Null,
    Str(String),
    Date(f64),
    Num(f64),
    Array(Vec<IdbKey>),
    Binary(Vec<u8>),
    MinKey,
}

impl std::fmt::Display for IdbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdbKey::Null => write!(f, "null"),
            IdbKey::Str(s) => write!(f, "{s}"),
            IdbKey::Date(d) => write!(f, "Date({d})"),
            IdbKey::Num(n) => write!(f, "{n}"),
            IdbKey::Array(a) => {
                write!(f, "[")?;
                for (i, k) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}")?;
                }
                write!(f, "]")
            }
            IdbKey::Binary(b) => write!(f, "0x{}", hex::encode(b)),
            IdbKey::MinKey => write!(f, "minkey"),
        }
    }
}

pub fn decode_idb_key(b: &[u8], pos: &mut usize) -> Result<IdbKey> {
    let t = *b.get(*pos).context("empty key")?;
    *pos += 1;
    Ok(match t {
        0 => IdbKey::Null,
        1 => IdbKey::Str(string_with_length(b, pos)?),
        2 | 3 => {
            let d = b.get(*pos..*pos + 8).context("truncated double")?;
            *pos += 8;
            let v = f64::from_le_bytes(d.try_into().unwrap());
            if t == 2 { IdbKey::Date(v) } else { IdbKey::Num(v) }
        }
        4 => {
            let n = read_varint(b, pos)?;
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(decode_idb_key(b, pos)?);
            }
            IdbKey::Array(v)
        }
        5 => IdbKey::MinKey,
        6 => {
            let n = read_varint(b, pos)? as usize;
            let d = b.get(*pos..*pos + n).context("truncated binary")?;
            *pos += n;
            IdbKey::Binary(d.to_vec())
        }
        t => bail!("unknown idb key type {t}"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Newest version of this key, not deleted.
    Live,
    /// An older version of a key that was later overwritten.
    Superseded,
    /// A version of a key whose newest entry is a deletion.
    Deleted,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Live => "live",
            Status::Superseded => "superseded",
            Status::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Database {
    pub id: u64,
    pub origin: String,
    pub name: String,
    pub stores: BTreeMap<u64, String>,
}

/// One decoded object-store record (one version of one key).
#[derive(Debug, Clone)]
pub struct Record {
    pub db: u64,
    pub store: u64,
    pub key: IdbKey,
    pub seq: u64,
    pub status: Status,
    pub file: usize,
    pub origin: leveldb::Origin,
    pub crc_ok: bool,
    pub value: Result<JsValue, String>,
}

/// Database / object-store names plus per-file parse statistics.
pub struct Catalog {
    pub files: Vec<leveldb::FileStats>,
    pub databases: BTreeMap<u64, Database>,
}

impl Catalog {
    pub fn store_name(&self, db: u64, store: u64) -> &str {
        self.databases.get(&db).and_then(|d| d.stores.get(&store)).map(|s| s.as_str()).unwrap_or("?")
    }
    pub fn db_name(&self, db: u64) -> &str {
        self.databases.get(&db).map(|d| d.name.as_str()).unwrap_or("?")
    }
}

/// Pass 1: read database and object-store names (newest version of each key).
pub fn catalog(leveldb_dir: &Path) -> Result<Catalog> {
    let mut newest: HashMap<Vec<u8>, (u64, bool, Vec<u8>)> = HashMap::new();
    let files = leveldb::scan_dir(leveldb_dir, &mut |r| {
        let Some((p, _)) = decode_prefix(r.key) else { return };
        if !(p.db == 0 || p.store == 0) {
            return;
        }
        match newest.get(r.key) {
            Some((seq, _, _)) if *seq > r.seq => {}
            _ => {
                newest.insert(r.key.to_vec(), (r.seq, r.deleted, r.value.to_vec()));
            }
        }
    })?;
    let mut databases: BTreeMap<u64, Database> = BTreeMap::new();
    let mut store_names: Vec<(u64, u64, String)> = Vec::new();
    for (key, (_, deleted, value)) in &newest {
        if *deleted {
            continue;
        }
        let Some((p, rest)) = decode_prefix(key) else { continue };
        if p.db == 0 && p.store == 0 && p.index == 0 {
            // Global metadata. 201 = DatabaseName: origin, name -> id
            if rest.first() == Some(&201) {
                let mut pos = 1;
                let (Ok(origin), Ok(name)) = (string_with_length(rest, &mut pos), string_with_length(rest, &mut pos))
                else {
                    continue;
                };
                let id = read_le_int(value);
                databases.insert(id, Database { id, origin, name, stores: BTreeMap::new() });
            }
        } else if p.db != 0 && p.store == 0 && p.index == 0 {
            // Database metadata. 50 = ObjectStoreMetaData: varint store id, type byte (0 = name)
            if rest.first() == Some(&50) {
                let mut pos = 1;
                let Ok(sid) = read_varint(rest, &mut pos) else { continue };
                if rest.get(pos) == Some(&0) {
                    store_names.push((p.db, sid, utf16be(value)));
                }
            }
        }
    }
    for (db, sid, name) in store_names {
        if let Some(d) = databases.get_mut(&db) {
            d.stores.insert(sid, name);
        }
    }
    Ok(Catalog { files, databases })
}

/// The BlobEntry key for an object-store data key: same key, index id 3 instead of 1.
fn blob_entry_key(data_key: &[u8]) -> Option<Vec<u8>> {
    let first = *data_key.first()?;
    if first & 0x3 != 0 {
        return None; // index id is not a single byte, so it can't be 1
    }
    let db_len = ((first >> 5) & 0x7) as usize + 1;
    let os_len = ((first >> 2) & 0x7) as usize + 1;
    let at = 1 + db_len + os_len;
    let mut k = data_key.to_vec();
    *k.get_mut(at)? = BLOB_ENTRY as u8;
    Some(k)
}

/// Decode every version of every record in the object stores selected by
/// `want(database, store_name)`, handing them to `sink` one at a time.
///
/// Two passes over the files: the first learns, for each key, the newest
/// sequence number (to classify live / superseded / deleted) and the blob
/// lists; the second decodes values.
pub fn scan_records(
    leveldb_dir: &Path,
    blob_dir: Option<&Path>,
    cat: &Catalog,
    want: &dyn Fn(&Database, &str) -> bool,
    sink: &mut dyn FnMut(Record),
) -> Result<()> {
    let selected: HashSet<(u64, u64)> = cat
        .databases
        .values()
        .flat_map(|d| d.stores.iter().filter(|(_, sn)| want(d, sn)).map(move |(sid, _)| (d.id, *sid)))
        .collect();
    let is_selected =
        |p: &Prefix| (p.index == OBJECT_STORE_DATA || p.index == BLOB_ENTRY) && selected.contains(&(p.db, p.store));

    // Pass A: newest (seq, deleted) per key; newest blob list per key.
    let mut newest: HashMap<Vec<u8>, (u64, bool)> = HashMap::new();
    let mut blob_lists: HashMap<Vec<u8>, (u64, Vec<u8>)> = HashMap::new();
    leveldb::scan_dir(leveldb_dir, &mut |r| {
        let Some((p, _)) = decode_prefix(r.key) else { return };
        if !is_selected(&p) {
            return;
        }
        let e = newest.entry(r.key.to_vec()).or_insert((0, false));
        if r.seq >= e.0 {
            *e = (r.seq, r.deleted);
        }
        if p.index == BLOB_ENTRY && !r.deleted {
            let e = blob_lists.entry(r.key.to_vec()).or_insert((0, Vec::new()));
            if r.seq >= e.0 {
                *e = (r.seq, r.value.to_vec());
            }
        }
    })?;

    // Pass B: decode.
    leveldb::scan_dir(leveldb_dir, &mut |r| {
        if r.deleted {
            return; // tombstones carry no value; their effect is in `status`
        }
        let Some((p, rest)) = decode_prefix(r.key) else { return };
        if p.index != OBJECT_STORE_DATA || !selected.contains(&(p.db, p.store)) {
            return;
        }
        let (nseq, ndel) = newest.get(r.key).copied().unwrap_or((r.seq, false));
        let status = if ndel {
            Status::Deleted
        } else if r.seq >= nseq {
            Status::Live
        } else {
            Status::Superseded
        };
        let mut kp = 0;
        let key = decode_idb_key(rest, &mut kp).unwrap_or_else(|_| IdbKey::Binary(rest.to_vec()));
        let blobs = blob_entry_key(r.key).and_then(|k| blob_lists.get(&k)).map(|(_, v)| v.as_slice());
        let value = decode_value(r.value, p.db, blobs, blob_dir).map_err(|e| format!("{e:#}"));
        sink(Record {
            db: p.db,
            store: p.store,
            key,
            seq: r.seq,
            status,
            file: r.file,
            origin: r.origin,
            crc_ok: r.crc_ok,
            value,
        });
    })?;
    Ok(())
}

// ------------------------------------------------------------ value decoding

const SSV_VERSION_TAG: u8 = 0xFF;
const REQUIRES_PROCESSING: u8 = 0x11;
const REPLACE_WITH_BLOB: u8 = 0x01;
const COMPRESSED_WITH_SNAPPY: u8 = 0x02;

pub struct BlobInfo {
    pub number: u64,
    pub mime: String,
    pub size: u64,
}

fn parse_blob_infos(b: &[u8]) -> Result<Vec<BlobInfo>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < b.len() {
        let kind = b[pos];
        pos += 1;
        match kind {
            0 | 1 => {
                let number = read_varint(b, &mut pos)?;
                let mime = string_with_length(b, &mut pos)?;
                let size = read_varint(b, &mut pos)?;
                if kind == 1 {
                    let _name = string_with_length(b, &mut pos)?;
                    let _mtime = read_varint(b, &mut pos)?;
                }
                out.push(BlobInfo { number, mime, size });
            }
            k => bail!("unsupported external object kind {k}"),
        }
    }
    Ok(out)
}

pub fn blob_path(blob_dir: &Path, db: u64, number: u64) -> PathBuf {
    blob_dir.join(format!("{db:x}")).join(format!("{:02x}", (number >> 8) & 0xff)).join(format!("{number:x}"))
}

fn decode_value(raw: &[u8], db: u64, blobs: Option<&[u8]>, blob_dir: Option<&Path>) -> Result<JsValue> {
    let mut pos = 0;
    let _version = read_varint(raw, &mut pos)?; // object store data version
    let mut ssv: Vec<u8> = raw[pos..].to_vec();
    for _ in 0..4 {
        if ssv.len() >= 3 && ssv[0] == SSV_VERSION_TAG && ssv[1] == REQUIRES_PROCESSING {
            match ssv[2] {
                COMPRESSED_WITH_SNAPPY => {
                    ssv = snap::raw::Decoder::new().decompress_vec(&ssv[3..]).context("snappy-wrapped value")?;
                }
                REPLACE_WITH_BLOB => {
                    let mut p = 3;
                    let _size = read_varint(&ssv, &mut p)?;
                    let index = read_varint(&ssv, &mut p)? as usize;
                    let infos = parse_blob_infos(blobs.context("value stored in blob but no blob entry")?)?;
                    let info = infos.get(index).context("blob index out of range")?;
                    let dir = blob_dir.context("value stored in blob but no blob dir")?;
                    let path = blob_path(dir, db, info.number);
                    ssv = std::fs::read(&path).with_context(|| format!("reading blob {}", path.display()))?;
                }
                t => bail!("unknown wrapping tag {t}"),
            }
        } else {
            break;
        }
    }
    let mut v = v8::deserialize_blink(&ssv)?;
    if let Some(b) = blobs
        && let Ok(infos) = parse_blob_infos(b)
    {
        resolve_blob_refs(&mut v, db, &infos);
    }
    Ok(v)
}

/// Replace Blink blob/file references with a description of the blob file
/// (path relative to the `.blob` directory), so they survive into JSON.
fn resolve_blob_refs(v: &mut JsValue, db: u64, infos: &[BlobInfo]) {
    match v {
        JsValue::BlobRef(i) => {
            if let Some(info) = infos.get(*i as usize) {
                let rel = format!("{db:x}/{:02x}/{:x}", (info.number >> 8) & 0xff, info.number);
                *v = JsValue::Object(vec![
                    ("$blobFile".into(), JsValue::Str(rel)),
                    ("type".into(), JsValue::Str(info.mime.clone())),
                    ("size".into(), JsValue::Int(info.size as i64)),
                ]);
            }
        }
        JsValue::Array(a) | JsValue::Set(a) => a.iter_mut().for_each(|x| resolve_blob_refs(x, db, infos)),
        JsValue::Object(p) => p.iter_mut().for_each(|(_, x)| resolve_blob_refs(x, db, infos)),
        JsValue::Map(m) => m.iter_mut().for_each(|(_, x)| resolve_blob_refs(x, db, infos)),
        _ => {}
    }
}
