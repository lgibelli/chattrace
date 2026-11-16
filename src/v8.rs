// SPDX-License-Identifier: GPL-3.0-or-later
//! Deserializer for Blink SerializedScriptValue / V8 ValueSerializer data
//! (v8/src/objects/value-serializer.cc), as stored in IndexedDB.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value as Json};

#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Hole,
    Bool(bool),
    Int(i64),
    Num(f64),
    BigInt(String),
    Str(String),
    Date(f64),
    Array(Vec<JsValue>),
    Object(Vec<(String, JsValue)>),
    Map(Vec<(JsValue, JsValue)>),
    Set(Vec<JsValue>),
    Bytes(Vec<u8>),
    RegExp(String, u64),
    Error(String),
    /// Blink Blob/File stored as an external IndexedDB blob (index into the record's blob list).
    BlobRef(u32),
    /// Something we can't represent (host objects, wasm, …).
    Opaque(String),
}

impl JsValue {
    pub fn to_json(&self) -> Json {
        match self {
            JsValue::Undefined | JsValue::Hole => Json::Null,
            JsValue::Null => Json::Null,
            JsValue::Bool(b) => Json::Bool(*b),
            JsValue::Int(i) => Json::from(*i),
            JsValue::Num(n) => serde_json::Number::from_f64(*n).map(Json::Number).unwrap_or(Json::Null),
            JsValue::BigInt(s) => Json::String(s.clone()),
            JsValue::Str(s) => Json::String(s.clone()),
            JsValue::Date(d) => Json::String(fmt_ms(*d)),
            JsValue::Array(a) | JsValue::Set(a) => Json::Array(a.iter().map(|v| v.to_json()).collect()),
            JsValue::Object(props) => {
                let mut m = Map::new();
                for (k, v) in props {
                    m.insert(k.clone(), v.to_json());
                }
                Json::Object(m)
            }
            JsValue::Map(entries) => {
                let all_str = entries.iter().all(|(k, _)| matches!(k, JsValue::Str(_)));
                if all_str {
                    let mut m = Map::new();
                    for (k, v) in entries {
                        if let JsValue::Str(s) = k {
                            m.insert(s.clone(), v.to_json());
                        }
                    }
                    Json::Object(m)
                } else {
                    Json::Array(entries.iter().map(|(k, v)| Json::Array(vec![k.to_json(), v.to_json()])).collect())
                }
            }
            JsValue::Bytes(b) => Json::String(format!("base16:{}", hex::encode(b))),
            JsValue::RegExp(p, f) => Json::String(format!("/{p}/{f}")),
            JsValue::Error(m) => Json::String(format!("Error: {m}")),
            JsValue::BlobRef(i) => Json::String(format!("<blob #{i}>")),
            JsValue::Opaque(s) => Json::String(format!("<{s}>")),
        }
    }
}

pub fn fmt_ms(ms: f64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| ms.to_string())
}

/// Strip the Blink envelope (version tag, optional trailer offset) and decode
/// the V8 payload.
pub fn deserialize_blink(buf: &[u8]) -> Result<JsValue> {
    let mut d = Deserializer { buf, pos: 0, objects: Vec::new(), version: 0 };
    // Blink header: 0xFF <varint blink version> [0xFE <u64 offset> <u32 size>]
    if d.peek() == Some(0xFF) {
        d.pos += 1;
        let blink_version = d.varint()?;
        if blink_version >= 21 && d.peek() == Some(0xFE) {
            d.pos += 1 + 8 + 4;
        }
    }
    // V8 header: 0xFF <varint v8 version>
    d.skip_padding();
    if d.peek() == Some(0xFF) {
        d.pos += 1;
        d.version = d.varint()? as u32;
    }
    d.value(0)
}

struct Deserializer<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Objects by id for back-references; None while under construction.
    objects: Vec<Option<JsValue>>,
    version: u32,
}

const MAX_DEPTH: usize = 200;

impl<'a> Deserializer<'a> {
    fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    fn byte(&mut self) -> Result<u8> {
        let b = *self.buf.get(self.pos).context("unexpected end of value")?;
        self.pos += 1;
        Ok(b)
    }

    fn varint(&mut self) -> Result<u64> {
        crate::leveldb::read_varint(self.buf, &mut self.pos)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len()).context("truncated value")?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn double(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    fn skip_padding(&mut self) {
        while self.peek() == Some(0) {
            self.pos += 1;
        }
    }

    fn tag(&mut self) -> Result<u8> {
        self.skip_padding();
        self.byte()
    }

    fn peek_tag(&mut self) -> Option<u8> {
        self.skip_padding();
        self.peek()
    }

    fn new_id(&mut self) -> usize {
        self.objects.push(None);
        self.objects.len() - 1
    }

    fn done(&mut self, id: usize, v: JsValue) -> JsValue {
        self.objects[id] = Some(v.clone());
        v
    }

    fn one_byte_string(&mut self) -> Result<String> {
        let n = self.varint()? as usize;
        Ok(self.bytes(n)?.iter().map(|&b| b as char).collect())
    }

    fn two_byte_string(&mut self) -> Result<String> {
        let n = self.varint()? as usize;
        let b = self.bytes(n)?;
        let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(String::from_utf16_lossy(&units))
    }

    fn utf8_string(&mut self) -> Result<String> {
        let n = self.varint()? as usize;
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }

    fn string_tagged(&mut self) -> Result<String> {
        match self.value_depthless()? {
            JsValue::Str(s) => Ok(s),
            other => bail!("expected string, got {other:?}"),
        }
    }

    fn value_depthless(&mut self) -> Result<JsValue> {
        self.value(MAX_DEPTH - 1)
    }

    fn key_string(&mut self, depth: usize) -> Result<String> {
        Ok(match self.value(depth + 1)? {
            JsValue::Str(s) => s,
            JsValue::Int(i) => i.to_string(),
            JsValue::Num(n) if n.fract() == 0.0 => format!("{}", n as i64),
            JsValue::Num(n) => n.to_string(),
            other => format!("{other:?}"),
        })
    }

    /// Read `key, value` pairs until `end` tag; returns properties.
    fn properties(&mut self, end: u8, depth: usize) -> Result<Vec<(String, JsValue)>> {
        let mut props = Vec::new();
        loop {
            if self.peek_tag() == Some(end) {
                self.pos += 1;
                return Ok(props);
            }
            let k = self.key_string(depth)?;
            let v = self.value(depth + 1)?;
            props.push((k, v));
        }
    }

    fn value(&mut self, depth: usize) -> Result<JsValue> {
        if depth > MAX_DEPTH {
            bail!("value nested too deeply");
        }
        let tag = self.tag()?;
        Ok(match tag {
            b'_' => JsValue::Undefined,
            b'0' => JsValue::Null,
            b'-' => JsValue::Hole,
            b'T' => JsValue::Bool(true),
            b'F' => JsValue::Bool(false),
            b'I' => {
                let n = self.varint()?;
                JsValue::Int(((n >> 1) as i64) ^ -((n & 1) as i64))
            }
            b'U' => JsValue::Int(self.varint()? as i64),
            b'N' => JsValue::Num(self.double()?),
            b'Z' => JsValue::BigInt(self.bigint()?),
            b'"' => JsValue::Str(self.one_byte_string()?),
            b'c' => JsValue::Str(self.two_byte_string()?),
            b'S' => JsValue::Str(self.utf8_string()?),
            b'^' => {
                let id = self.varint()? as usize;
                match self.objects.get(id) {
                    Some(Some(v)) => v.clone(),
                    Some(None) => JsValue::Opaque("circular reference".into()),
                    None => bail!("bad object reference {id}"),
                }
            }
            b'o' => {
                let id = self.new_id();
                let props = self.properties(b'{', depth)?;
                let _n = self.varint()?;
                self.done(id, JsValue::Object(props))
            }
            b'A' => {
                let id = self.new_id();
                let len = self.varint()? as usize;
                let mut items = Vec::with_capacity(len.min(4096));
                for _ in 0..len {
                    items.push(self.value(depth + 1)?);
                }
                let extra = self.properties(b'$', depth)?;
                let _np = self.varint()?;
                let _len = self.varint()?;
                let v = if extra.is_empty() {
                    JsValue::Array(items)
                } else {
                    let mut props: Vec<(String, JsValue)> =
                        items.into_iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect();
                    props.extend(extra);
                    JsValue::Object(props)
                };
                self.done(id, v)
            }
            b'a' => {
                let id = self.new_id();
                let len = self.varint()? as usize;
                let props = self.properties(b'@', depth)?;
                let _np = self.varint()?;
                let _len = self.varint()?;
                // Render as dense array when all keys are indices < len.
                let mut arr = vec![JsValue::Hole; len.min(1 << 20)];
                let mut dense = true;
                let mut rest = Vec::new();
                for (k, v) in props {
                    match k.parse::<usize>() {
                        Ok(i) if i < arr.len() => arr[i] = v,
                        _ => {
                            dense = false;
                            rest.push((k, v));
                        }
                    }
                }
                let v = if dense {
                    JsValue::Array(arr)
                } else {
                    let mut props: Vec<(String, JsValue)> = arr
                        .into_iter()
                        .enumerate()
                        .filter(|(_, v)| *v != JsValue::Hole)
                        .map(|(i, v)| (i.to_string(), v))
                        .collect();
                    props.extend(rest);
                    JsValue::Object(props)
                };
                self.done(id, v)
            }
            b'D' => {
                let id = self.new_id();
                let d = self.double()?;
                self.done(id, JsValue::Date(d))
            }
            b'y' | b'x' => {
                let id = self.new_id();
                self.done(id, JsValue::Bool(tag == b'y'))
            }
            b'n' => {
                let id = self.new_id();
                let d = self.double()?;
                self.done(id, JsValue::Num(d))
            }
            b'z' => {
                let id = self.new_id();
                let s = self.bigint()?;
                self.done(id, JsValue::BigInt(s))
            }
            b's' => {
                let id = self.new_id();
                let s = self.string_tagged()?;
                self.done(id, JsValue::Str(s))
            }
            b'R' => {
                let id = self.new_id();
                let p = self.string_tagged()?;
                let flags = self.varint()?;
                self.done(id, JsValue::RegExp(p, flags))
            }
            b';' => {
                let id = self.new_id();
                let mut entries = Vec::new();
                loop {
                    if self.peek_tag() == Some(b':') {
                        self.pos += 1;
                        break;
                    }
                    let k = self.value(depth + 1)?;
                    let v = self.value(depth + 1)?;
                    entries.push((k, v));
                }
                let _n = self.varint()?;
                self.done(id, JsValue::Map(entries))
            }
            b'\'' => {
                let id = self.new_id();
                let mut items = Vec::new();
                loop {
                    if self.peek_tag() == Some(b',') {
                        self.pos += 1;
                        break;
                    }
                    items.push(self.value(depth + 1)?);
                }
                let _n = self.varint()?;
                self.done(id, JsValue::Set(items))
            }
            b'B' => {
                let id = self.new_id();
                let n = self.varint()? as usize;
                let b = self.bytes(n)?.to_vec();
                let v = self.maybe_view(JsValue::Bytes(b))?;
                self.done(id, v)
            }
            b'~' => {
                let id = self.new_id();
                let n = self.varint()? as usize;
                let _max = self.varint()?;
                let b = self.bytes(n)?.to_vec();
                let v = self.maybe_view(JsValue::Bytes(b))?;
                self.done(id, v)
            }
            b'r' => {
                let id = self.new_id();
                let mut msg = String::new();
                loop {
                    match self.byte()? {
                        b'm' => msg = self.string_tagged()?,
                        b's' => {
                            let _stack = self.string_tagged()?;
                        }
                        b'c' => {
                            let _cause = self.value(depth + 1)?;
                        }
                        b'.' => break,
                        _ => {} // error prototype subtags
                    }
                }
                self.done(id, JsValue::Error(msg))
            }
            b'\\' => {
                // Blink host objects (third_party/blink/.../serialization_tag.h)
                let id = self.new_id();
                let v = match self.byte()? {
                    b'i' | b'e' => JsValue::BlobRef(self.varint()? as u32), // blob / file index
                    b'b' => {
                        let uuid = self.utf8_string()?;
                        let mime = self.utf8_string()?;
                        let size = self.varint()?;
                        JsValue::Opaque(format!("blob {uuid} {mime} {size} bytes"))
                    }
                    t => bail!("unsupported Blink host object tag 0x{t:02x}"),
                };
                self.done(id, v)
            }
            b'?' => {
                let _count = self.varint()?;
                self.value(depth)?
            }
            t => bail!("unsupported V8 tag 0x{t:02x} at {}", self.pos - 1),
        })
    }

    fn maybe_view(&mut self, buffer: JsValue) -> Result<JsValue> {
        if self.peek() != Some(b'V') {
            return Ok(buffer);
        }
        self.pos += 1;
        let _sub = self.byte()?;
        let off = self.varint()? as usize;
        let len = self.varint()? as usize;
        if self.version >= 14 {
            let _flags = self.varint()?;
        }
        let JsValue::Bytes(b) = buffer else { unreachable!() };
        let slice = b.get(off..off + len).map(|s| s.to_vec()).unwrap_or_default();
        Ok(JsValue::Bytes(slice))
    }

    fn bigint(&mut self) -> Result<String> {
        let bitfield = self.varint()?;
        let neg = bitfield & 1 == 1;
        let n = (bitfield >> 1) as usize;
        let digits = self.bytes(n)?;
        if n <= 16 {
            let mut v: u128 = 0;
            for &d in digits.iter().rev() {
                v = (v << 8) | d as u128;
            }
            Ok(format!("{}{}", if neg { "-" } else { "" }, v))
        } else {
            let mut le = digits.to_vec();
            le.reverse();
            Ok(format!("{}0x{}", if neg { "-" } else { "" }, hex::encode(le)))
        }
    }
}
