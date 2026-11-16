// SPDX-License-Identifier: GPL-3.0-or-later
//! Minimal, read-only LevelDB file reader.
//!
//! This deliberately does NOT use a LevelDB library: opening a database with
//! one takes the LOCK file, replays the log and may compact, i.e. it writes to
//! the evidence. Here every `.log` and `.ldb`/`.sst` file is parsed directly
//! and *every* record found is returned, including superseded versions and
//! deletion tombstones that a normal LevelDB read would hide.

use anyhow::{Context, Result, bail};
use std::path::Path;

const LOG_BLOCK_SIZE: usize = 32 * 1024;
const LOG_HEADER_SIZE: usize = 7;
const TABLE_MAGIC: u64 = 0xdb4775248b80fb57;
const FOOTER_SIZE: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Origin {
    /// Write-ahead log; `offset` is the byte offset of the first fragment of the batch.
    Log { offset: u64 },
    /// Sorted table; `offset` is the byte offset of the data block.
    Table { offset: u64 },
}

/// One record as found on disk (borrowed from the file buffer).
#[derive(Debug, Clone, Copy)]
pub struct RawRecord<'a> {
    /// Index into the returned `Vec<FileStats>`.
    pub file: usize,
    pub origin: Origin,
    pub seq: u64,
    pub deleted: bool,
    pub key: &'a [u8],
    pub value: &'a [u8],
    /// CRC of the containing log record / table block verified OK.
    pub crc_ok: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FileStats {
    pub name: String,
    pub records: usize,
    pub crc_errors: usize,
    pub errors: Vec<String>,
}

pub type Sink<'s> = dyn FnMut(RawRecord<'_>) + 's;

/// Parse every log and table file in `dir`, handing each record to `sink`.
/// Files are opened read-only and processed in name order.
pub fn scan_dir(dir: &Path, sink: &mut Sink<'_>) -> Result<Vec<FileStats>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".log") || n.ends_with(".ldb") || n.ends_with(".sst"))
        .collect();
    names.sort();

    let mut files = Vec::new();
    for name in names {
        let idx = files.len();
        let mut stats = FileStats { name: name.clone(), ..Default::default() };
        let data = std::fs::read(dir.join(&name)).with_context(|| format!("reading {name}"))?;
        let mut count = 0usize;
        let mut counting = |r: RawRecord<'_>| {
            count += 1;
            sink(r)
        };
        let res = if name.ends_with(".log") {
            read_log(&data, idx, &mut counting, &mut stats)
        } else {
            read_table(&data, idx, &mut counting, &mut stats)
        };
        if let Err(e) = res {
            stats.errors.push(format!("{e:#}"));
        }
        stats.records = count;
        files.push(stats);
    }
    Ok(files)
}

// ---------------------------------------------------------------- varints

pub fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let Some(&b) = buf.get(*pos) else { bail!("truncated varint") };
        *pos += 1;
        if shift >= 64 {
            bail!("varint too long");
        }
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
}

fn take<'a>(buf: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8]> {
    let end = pos.checked_add(n).filter(|&e| e <= buf.len());
    let Some(end) = end else { bail!("truncated: need {n} bytes at {}", *pos) };
    let s = &buf[*pos..end];
    *pos = end;
    Ok(s)
}

// ------------------------------------------------------------------- crc32c

fn crc32c_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0x82F6_3B78 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

fn crc32c(parts: &[&[u8]]) -> u32 {
    let t = crc32c_table();
    let mut c = !0u32;
    for p in parts {
        for &b in *p {
            c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
    }
    !c
}

fn unmask_crc(masked: u32) -> u32 {
    let rot = masked.wrapping_sub(0xa282_ead8);
    rot.rotate_left(15)
}

// --------------------------------------------------------------------- log

fn read_log(data: &[u8], file: usize, sink: &mut Sink<'_>, stats: &mut FileStats) -> Result<()> {
    let mut pos = 0usize;
    let mut payload: Vec<u8> = Vec::new();
    let mut payload_start = 0u64;
    let mut payload_crc_ok = true;
    let mut in_fragmented = false;

    while pos < data.len() {
        let block_left = LOG_BLOCK_SIZE - (pos % LOG_BLOCK_SIZE);
        if block_left < LOG_HEADER_SIZE {
            pos += block_left; // trailer padding
            continue;
        }
        if pos + LOG_HEADER_SIZE > data.len() {
            break;
        }
        let h = &data[pos..pos + LOG_HEADER_SIZE];
        let masked = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let len = u16::from_le_bytes([h[4], h[5]]) as usize;
        let typ = h[6];
        let rec_start = pos;
        pos += LOG_HEADER_SIZE;
        if typ == 0 && len == 0 {
            // pre-allocated / zeroed space: skip rest of block
            pos = rec_start + block_left;
            continue;
        }
        if pos + len > data.len() || len > block_left - LOG_HEADER_SIZE {
            stats.errors.push(format!("truncated log record at {rec_start}"));
            break;
        }
        let frag = &data[pos..pos + len];
        pos += len;
        let crc_ok = unmask_crc(masked) == crc32c(&[&[typ], frag]);
        if !crc_ok {
            stats.crc_errors += 1;
        }
        match typ {
            1 => {
                // FULL
                parse_batch(frag, rec_start as u64, crc_ok, file, sink, stats);
                in_fragmented = false;
            }
            2 => {
                // FIRST
                payload.clear();
                payload.extend_from_slice(frag);
                payload_start = rec_start as u64;
                payload_crc_ok = crc_ok;
                in_fragmented = true;
            }
            3 | 4 => {
                if !in_fragmented {
                    stats.errors.push(format!("orphan log fragment at {rec_start}"));
                    continue;
                }
                payload.extend_from_slice(frag);
                payload_crc_ok &= crc_ok;
                if typ == 4 {
                    let p = std::mem::take(&mut payload);
                    parse_batch(&p, payload_start, payload_crc_ok, file, sink, stats);
                    in_fragmented = false;
                }
            }
            other => stats.errors.push(format!("unknown log record type {other} at {rec_start}")),
        }
    }
    if in_fragmented {
        stats.errors.push("log ends inside a fragmented record".into());
    }
    Ok(())
}

fn parse_batch(b: &[u8], offset: u64, crc_ok: bool, file: usize, sink: &mut Sink<'_>, stats: &mut FileStats) {
    let res: Result<()> = (|| {
        if b.len() < 12 {
            bail!("short write batch");
        }
        let seq = u64::from_le_bytes(b[0..8].try_into().unwrap());
        let count = u32::from_le_bytes(b[8..12].try_into().unwrap()) as u64;
        let mut pos = 12;
        for i in 0..count {
            let tag = *b.get(pos).context("truncated batch")?;
            pos += 1;
            let klen = read_varint(b, &mut pos)? as usize;
            let key = take(b, &mut pos, klen)?;
            let (deleted, value) = match tag {
                1 => {
                    let vlen = read_varint(b, &mut pos)? as usize;
                    (false, take(b, &mut pos, vlen)?)
                }
                0 => (true, &b[0..0]),
                t => bail!("unknown batch tag {t}"),
            };
            sink(RawRecord { file, origin: Origin::Log { offset }, seq: seq + i, deleted, key, value, crc_ok });
        }
        Ok(())
    })();
    if let Err(e) = res {
        stats.errors.push(format!("batch at {offset}: {e:#}"));
    }
}

// ------------------------------------------------------------------- table

struct BlockHandle {
    offset: u64,
    size: u64,
}

fn read_handle(buf: &[u8], pos: &mut usize) -> Result<BlockHandle> {
    Ok(BlockHandle { offset: read_varint(buf, pos)?, size: read_varint(buf, pos)? })
}

/// Returns (block contents, crc_ok).
fn read_block(data: &[u8], h: &BlockHandle) -> Result<(Vec<u8>, bool)> {
    let start = h.offset as usize;
    let end = start + h.size as usize;
    if end + 5 > data.len() {
        bail!("block handle out of range");
    }
    let raw = &data[start..end];
    let ctype = data[end];
    let masked = u32::from_le_bytes(data[end + 1..end + 5].try_into().unwrap());
    let crc_ok = unmask_crc(masked) == crc32c(&[raw, &[ctype]]);
    let contents = match ctype {
        0 => raw.to_vec(),
        1 => snap::raw::Decoder::new().decompress_vec(raw).context("snappy decompress")?,
        t => bail!("unsupported block compression type {t}"),
    };
    Ok((contents, crc_ok))
}

/// Iterate (key, value) entries of a decoded block.
fn block_entries(block: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    if block.len() < 4 {
        bail!("short block");
    }
    let n_restarts = u32::from_le_bytes(block[block.len() - 4..].try_into().unwrap()) as usize;
    let limit = block.len().checked_sub(4 + 4 * n_restarts).context("bad restart count")?;
    let mut out = Vec::new();
    let mut pos = 0;
    let mut key: Vec<u8> = Vec::new();
    while pos < limit {
        let shared = read_varint(block, &mut pos)? as usize;
        let non_shared = read_varint(block, &mut pos)? as usize;
        let vlen = read_varint(block, &mut pos)? as usize;
        if shared > key.len() {
            bail!("bad shared prefix");
        }
        key.truncate(shared);
        key.extend_from_slice(take(block, &mut pos, non_shared)?);
        let v = take(block, &mut pos, vlen)?;
        out.push((key.clone(), v.to_vec()));
    }
    Ok(out)
}

fn read_table(data: &[u8], file: usize, sink: &mut Sink<'_>, stats: &mut FileStats) -> Result<()> {
    if data.len() < FOOTER_SIZE {
        bail!("file too small for a table");
    }
    let footer = &data[data.len() - FOOTER_SIZE..];
    let magic = u64::from_le_bytes(footer[40..48].try_into().unwrap());
    if magic != TABLE_MAGIC {
        bail!("bad table magic");
    }
    let mut p = 0;
    let _meta = read_handle(footer, &mut p)?;
    let index_h = read_handle(footer, &mut p)?;
    let (index, index_crc) = read_block(data, &index_h)?;
    if !index_crc {
        stats.crc_errors += 1;
    }
    for (_sep, hv) in block_entries(&index)? {
        let mut hp = 0;
        let h = read_handle(&hv, &mut hp)?;
        let (block, crc_ok) = match read_block(data, &h) {
            Ok(b) => b,
            Err(e) => {
                stats.errors.push(format!("block at {}: {e:#}", h.offset));
                continue;
            }
        };
        if !crc_ok {
            stats.crc_errors += 1;
        }
        let entries = match block_entries(&block) {
            Ok(e) => e,
            Err(e) => {
                stats.errors.push(format!("block at {}: {e:#}", h.offset));
                continue;
            }
        };
        for (ikey, value) in entries {
            if ikey.len() < 8 {
                stats.errors.push(format!("short internal key in block {}", h.offset));
                continue;
            }
            let (ukey, trailer) = ikey.split_at(ikey.len() - 8);
            let t = u64::from_le_bytes(trailer.try_into().unwrap());
            sink(RawRecord {
                file,
                origin: Origin::Table { offset: h.offset },
                seq: t >> 8,
                deleted: (t & 0xff) == 0,
                key: ukey,
                value: &value,
                crc_ok,
            });
        }
    }
    Ok(())
}
