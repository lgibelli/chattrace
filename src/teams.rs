// SPDX-License-Identifier: GPL-3.0-or-later
//! Microsoft Teams (new client, "Teams 2") data model on top of IndexedDB.
//!
//! Relevant databases are named `Teams:<manager>:react-web-client:<tenant>:<user>:<locale>`:
//!   replychain-manager   / replychains, replychains-2  -> messages (grouped in reply chains)
//!   conversation-manager / conversations               -> chats, meetings, channels
//!   profiles             / profiles                    -> people (MRI -> name, e-mail)
//! A locale change creates a second set of databases, so the same message can
//! appear several times; versions are de-duplicated per account.

use crate::idb::{self, Catalog, Record, Status};
use crate::v8::fmt_ms;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    pub database: String,
    pub store: String,
    pub file: String,
    pub seq: u64,
    pub status: Status,
    pub crc_ok: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub key: String,
    pub tenant_id: String,
    pub user_id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub locales: Vec<String>,
}

impl Account {
    pub fn label(&self) -> String {
        match (&self.name, &self.email) {
            (Some(n), Some(e)) => format!("{n} <{e}>"),
            (Some(n), None) => n.clone(),
            (None, Some(e)) => e.clone(),
            _ => format!("user {} @ tenant {}", short(&self.user_id), short(&self.tenant_id)),
        }
    }
}

fn short(s: &str) -> &str {
    &s[..s.len().min(8)]
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub mri: String,
    pub display_name: String,
    pub email: Option<String>,
    pub upn: Option<String>,
    pub job_title: Option<String>,
    pub tenant_name: Option<String>,
    pub prov: Provenance,
}

#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub mri: String,
    pub role: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub account: String,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub created: Option<f64>,
    pub last_activity: Option<f64>,
    pub members: Vec<Member>,
    /// Times Teams purged cached reply chains for this conversation (documents gaps).
    pub cache_purges: Vec<Json>,
    pub message_count: usize,
    /// False when the conversation is only known from its messages.
    pub has_metadata: bool,
    pub raw: Json,
    pub prov: Option<Provenance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reaction {
    pub emoji: String,
    pub mri: String,
    pub name: String,
    pub time: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    /// Most recent version of the message found in the cache.
    Latest,
    /// An older state of the message (before an edit, deletion, reaction…).
    Earlier,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRef {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub account: String,
    pub conversation_id: String,
    pub id: String,
    pub version: String,
    pub client_message_id: String,
    pub reply_chain_id: String,
    pub sequence_id: Option<i64>,
    pub arrival: Option<f64>,
    pub client_arrival: Option<f64>,
    pub compose_time: Option<String>,
    pub sender_mri: String,
    pub sender_name: String,
    pub from_me: bool,
    pub message_type: String,
    pub content_type: String,
    pub content: String,
    pub text: String,
    pub edited: Option<f64>,
    pub deleted: Option<f64>,
    pub deleted_by: Option<String>,
    pub reactions: Vec<Reaction>,
    pub mentions: Vec<String>,
    pub files: Vec<FileRef>,
    pub importance: String,
    pub subject: String,
    pub variant: Variant,
    /// Number of distinct versions of this message found.
    pub versions_known: usize,
    /// For earlier versions: what differs from the latest version
    /// ("content", "messageType", "version", "properties.<key>").
    pub changes: Vec<String>,
    /// Earlier version that differs in something a reader would notice
    /// (text, edit/delete markers, reactions, files, mentions).
    pub substantive: bool,
    /// Number of cache records containing exactly this version.
    pub occurrences: usize,
    pub fingerprint: String,
    pub raw: Json,
    pub prov: Provenance,
}

impl Message {
    pub fn is_system(&self) -> bool {
        self.message_type.starts_with("ThreadActivity/")
            || self.message_type.starts_with("Event/")
            || self.message_type.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DecodeError {
    pub database: String,
    pub store: String,
    pub key: String,
    pub error: String,
}

pub struct Dataset {
    pub catalog: Catalog,
    pub accounts: Vec<Account>,
    pub profiles: BTreeMap<String, Profile>,
    pub conversations: Vec<Conversation>,
    pub messages: Vec<Message>,
    pub decode_errors: Vec<DecodeError>,
}

impl Dataset {
    pub fn account(&self, key: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.key == key)
    }
    pub fn conversation(&self, account: &str, id: &str) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.account == account && c.id == id)
    }
}

/// `Teams:<manager>:react-web-client:<tenant>:<user>:<locale>` -> (manager, tenant, user, locale)
pub fn parse_db_name(name: &str) -> Option<(&str, &str, &str, &str)> {
    let rest = name.strip_prefix("Teams:")?;
    let (manager, rest) = rest.split_once(":react-web-client:")?;
    let (tenant, rest) = rest.split_once(':')?;
    let (user, locale) = rest.rsplit_once(':')?;
    let user = user.strip_prefix("8:orgid:").unwrap_or(user);
    Some((manager, tenant, user, locale))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    ReplyChains,
    Conversations,
    Profiles,
}

fn store_kind(db_name: &str, store: &str) -> Option<Kind> {
    let (manager, ..) = parse_db_name(db_name)?;
    match (manager, store) {
        (m, s) if m.contains("replychain") && !m.contains("metadata") && s.starts_with("replychains") => {
            Some(Kind::ReplyChains)
        }
        (m, s) if m.contains("replychain") && !m.contains("metadata") && s == "threads" => Some(Kind::ReplyChains),
        ("conversation-manager", "conversations") => Some(Kind::Conversations),
        ("profiles", "profiles") => Some(Kind::Profiles),
        _ => None,
    }
}

pub fn blob_dir_for(leveldb_dir: &Path) -> Option<PathBuf> {
    crate::discover::blob_dir_for(leveldb_dir)
}

// ------------------------------------------------------------------ helpers

/// Teams stores many nested values as JSON strings; parse them transparently.
fn as_json(v: &Json) -> Json {
    match v {
        Json::String(s) if s.starts_with('[') || s.starts_with('{') => {
            serde_json::from_str(s).unwrap_or_else(|_| v.clone())
        }
        _ => v.clone(),
    }
}

fn s(v: &Json, key: &str) -> String {
    match v.get(key) {
        Some(Json::String(x)) => x.clone(),
        Some(Json::Number(n)) => n.as_f64().map(num_str).unwrap_or_default(),
        Some(Json::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn num_str(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e17 { format!("{}", n as i64) } else { n.to_string() }
}

fn opt(v: String) -> Option<String> {
    (!v.is_empty()).then_some(v)
}

/// Milliseconds since the epoch from a number, digit string or ISO-8601 string.
pub fn as_ms(v: Option<&Json>) -> Option<f64> {
    match v? {
        Json::Number(n) => n.as_f64().filter(|x| *x > 0.0),
        Json::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'.') => {
            s.parse::<f64>().ok().filter(|x| *x > 0.0)
        }
        Json::String(s) => chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis() as f64),
        _ => None,
    }
}

pub fn utc(ms: Option<f64>) -> String {
    ms.map(fmt_ms).unwrap_or_default()
}

pub fn local(ms: Option<f64>) -> String {
    ms.and_then(|m| chrono::DateTime::from_timestamp_millis(m as i64))
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string())
        .unwrap_or_default()
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            e if e.starts_with("#x") || e.starts_with("#X") => {
                u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32)
            }
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// HTML -> plain text (line breaks kept, tags dropped, entities decoded).
pub fn html_to_text(html: &str, tag_sep: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let tag = rest[i + 1..i + j].trim().to_ascii_lowercase();
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        let closing = tag.starts_with('/');
        match name.as_str() {
            "br" => out.push('\n'),
            "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "blockquote" if closing => out.push('\n'),
            "li" => out.push_str("• "),
            "img" if !closing => out.push_str("[image]"),
            "video" if !closing => out.push_str("[video]"),
            "audio" if !closing => out.push_str("[audio]"),
            _ => out.push_str(tag_sep),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    let text = decode_entities(&out).replace('\u{a0}', " ");
    // collapse runs of blank lines / spaces
    let mut lines: Vec<String> = text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}

/// Inner text of every `<tag>…</tag>` (case-insensitive, no nesting of the same tag).
fn xml_all(xml: &str, tag: &str) -> Vec<String> {
    let lower = xml.to_ascii_lowercase();
    let (open, close) = (format!("<{}", tag.to_ascii_lowercase()), format!("</{}>", tag.to_ascii_lowercase()));
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find(&open) {
        let start = from + i;
        let after = start + open.len();
        // must be the whole tag name
        if !matches!(lower.as_bytes().get(after), Some(b'>') | Some(b' ') | Some(b'/')) {
            from = after;
            continue;
        }
        let Some(gt) = lower[start..].find('>') else { break };
        let body = start + gt + 1;
        let Some(end) = lower[body..].find(&close) else { break };
        out.push(decode_entities(&xml[body..body + end]));
        from = body + end + close.len();
    }
    out
}

fn xml_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let i = lower.find(&format!("<{}", tag.to_ascii_lowercase()))?;
    let gt = lower[i..].find('>')? + i;
    let head = &xml[i..gt];
    let key = format!("{attr}=\"");
    let a = head.find(&key)? + key.len();
    let b = head[a..].find('"')? + a;
    Some(head[a..b].to_string())
}

fn readable_text(msg_type: &str, content: &str, props: &Json, names: &HashMap<String, String>) -> String {
    let name_of = |mri: &str| names.get(mri.trim()).cloned().unwrap_or_else(|| mri.trim().to_string());
    if let Some(cl) = props.get("call-log").map(as_json).filter(|v| v.is_object()) {
        let who = |k: &str, p: &str| {
            let n = cl.get(p).map(|x| s(x, "displayName")).unwrap_or_default();
            if n.is_empty() { name_of(&s(&cl, k)) } else { n }
        };
        return format!(
            "[call log] {} {} {}: {} → {} ({} – {})",
            s(&cl, "callDirection"),
            s(&cl, "callType"),
            s(&cl, "callState"),
            who("originator", "originatorParticipant"),
            who("target", "targetParticipant"),
            s(&cl, "startTime"),
            s(&cl, "endTime")
        );
    }
    if msg_type.starts_with("ThreadActivity/") {
        let label = msg_type.split('/').nth(1).unwrap_or(msg_type);
        let t = content.trim_start();
        let body = if t.starts_with('{') {
            let j: Json = serde_json::from_str(content).unwrap_or(Json::Null);
            let mut parts = Vec::new();
            if let Some(i) = j.get("initiator").and_then(|v| v.as_str()) {
                parts.push(format!("by {}", name_of(i)));
            }
            if let Some(ms) = j.get("members").and_then(|v| v.as_array()) {
                let list: Vec<String> = ms
                    .iter()
                    .map(|m| {
                        let n = s(m, "friendlyname");
                        if n.is_empty() { name_of(&s(m, "id")) } else { n }
                    })
                    .collect();
                parts.push(list.join(", "));
            }
            parts.join(": ")
        } else if t.starts_with('<') {
            let mut parts = Vec::new();
            if let Some(i) = xml_all(content, "initiator").first() {
                parts.push(format!("by {}", name_of(i)));
            }
            let targets: Vec<String> = xml_all(content, "target").iter().map(|t| name_of(t)).collect();
            if !targets.is_empty() {
                parts.push(targets.join(", "));
            }
            if let Some(v) = xml_all(content, "value").first() {
                parts.push(format!("\"{v}\""));
            }
            parts.join(": ")
        } else {
            content.to_string()
        };
        return format!("[{label}] {body}").trim().to_string();
    }
    if msg_type.starts_with("Event/") {
        let label = msg_type.split('/').nth(1).unwrap_or(msg_type);
        // e.g. <ended/><partlist type="ended"><part identity="…"><name>mri</name><displayName>…</displayName><duration>…</duration></part></partlist>
        let first_tag: String =
            content.trim_start().trim_start_matches('<').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        let kind = xml_attr(content, "partlist", "type")
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| if first_tag == "partlist" { String::new() } else { first_tag });
        let display = xml_all(content, "displayName");
        let who: Vec<String> = if display.iter().any(|d| !d.trim().is_empty()) {
            display.into_iter().filter(|d| !d.trim().is_empty()).collect()
        } else {
            xml_all(content, "name").iter().map(|n| name_of(n)).collect()
        };
        let dur = xml_all(content, "duration").iter().filter_map(|d| d.trim().parse::<f64>().ok()).fold(0.0, f64::max);
        let mut t = if kind.is_empty() && who.is_empty() {
            format!("[{label}] (no participant details)")
        } else {
            format!("[{label}] {kind}")
        };
        if !who.is_empty() {
            t.push_str(&format!(": {}", who.join(", ")));
        }
        if dur > 0.0 {
            t.push_str(&format!(" (duration {}m{:02}s)", (dur as u64) / 60, (dur as u64) % 60));
        }
        return t;
    }
    let mut text = html_to_text(content, "");
    if text.is_empty()
        && let Some(a) = props.get("activity").map(as_json).filter(|v| v.is_object())
    {
        text = format!(
            "[activity: {}] {}: {}",
            s(&a, "activityType"),
            s(&a, "sourceUserImDisplayName"),
            s(&a, "messagePreview")
        );
    }
    text
}

fn provenance(cat: &Catalog, r: &Record) -> Provenance {
    Provenance {
        database: cat.db_name(r.db).to_string(),
        store: cat.store_name(r.db, r.store).to_string(),
        file: cat.files.get(r.file).map(|f| f.name.clone()).unwrap_or_default(),
        seq: r.seq,
        status: r.status,
        crc_ok: r.crc_ok,
    }
}

fn status_rank(s: Status) -> u8 {
    match s {
        Status::Live => 2,
        Status::Superseded => 1,
        Status::Deleted => 0,
    }
}

// --------------------------------------------------------------------- load

struct RawMsg {
    account: String,
    json: Json,
    prov: Provenance,
}

pub fn load(leveldb_dir: &Path, blob_dir: Option<&Path>) -> Result<Dataset> {
    let catalog = idb::catalog(leveldb_dir)?;
    let mut raw_msgs: Vec<RawMsg> = Vec::new();
    let mut raw_convs: Vec<(String, Json, Provenance)> = Vec::new();
    let mut profiles: BTreeMap<String, Profile> = BTreeMap::new();
    let mut decode_errors = Vec::new();
    let mut accounts: BTreeMap<String, Account> = BTreeMap::new();

    for d in catalog.databases.values() {
        if let Some((_, tenant, user, locale)) = parse_db_name(&d.name) {
            let key = format!("{tenant}/{user}");
            let a = accounts.entry(key.clone()).or_insert_with(|| Account {
                key,
                tenant_id: tenant.into(),
                user_id: user.into(),
                name: None,
                email: None,
                locales: vec![],
            });
            if !a.locales.iter().any(|l| l == locale) {
                a.locales.push(locale.into());
            }
        }
    }

    idb::scan_records(leveldb_dir, blob_dir, &catalog, &|d, s| store_kind(&d.name, s).is_some(), &mut |r| {
        let db_name = catalog.db_name(r.db);
        let store = catalog.store_name(r.db, r.store);
        let Some(kind) = store_kind(db_name, store) else { return };
        let (_, tenant, user, _) = parse_db_name(db_name).unwrap();
        let account = format!("{tenant}/{user}");
        let prov = provenance(&catalog, &r);
        let v = match &r.value {
            Ok(v) => v.to_json(),
            Err(e) => {
                decode_errors.push(DecodeError {
                    database: prov.database.clone(),
                    store: prov.store.clone(),
                    key: r.key.to_string(),
                    error: e.clone(),
                });
                return;
            }
        };
        match kind {
            Kind::ReplyChains => {
                if let Some(map) = v.get("messageMap").and_then(|m| m.as_object()) {
                    for m in map.values() {
                        raw_msgs.push(RawMsg { account: account.clone(), json: m.clone(), prov: prov.clone() });
                    }
                }
            }
            Kind::Conversations => raw_convs.push((account, v, prov)),
            Kind::Profiles => {
                let mri = s(&v, "mri");
                let mri = if mri.is_empty() { r.key.to_string() } else { mri };
                let p = Profile {
                    mri: mri.clone(),
                    display_name: s(&v, "displayName"),
                    email: opt(s(&v, "email")).or_else(|| opt(s(&v, "mail"))),
                    upn: opt(s(&v, "userPrincipalName")),
                    job_title: opt(s(&v, "jobTitle")),
                    tenant_name: opt(s(&v, "tenantName")),
                    prov,
                };
                let replace = match profiles.get(&mri) {
                    None => true,
                    Some(old) => {
                        (status_rank(p.prov.status), p.prov.seq) > (status_rank(old.prov.status), old.prov.seq)
                    }
                };
                if replace {
                    profiles.insert(mri, p);
                }
            }
        }
    })?;

    // Names for MRIs: profiles, then conversation members, then message senders.
    let mut names: HashMap<String, String> = HashMap::new();
    for p in profiles.values() {
        if !p.display_name.is_empty() {
            names.insert(p.mri.clone(), p.display_name.clone());
        }
    }
    for (_, v, _) in &raw_convs {
        for m in v.get("members").and_then(|m| m.as_array()).into_iter().flatten() {
            let (id, n) = (s(m, "id"), s(m, "friendlyName"));
            if !n.is_empty() {
                names.entry(id).or_insert(n);
            }
        }
    }
    for m in &raw_msgs {
        let (id, n) = (s(&m.json, "creator"), s(&m.json, "imDisplayName"));
        if !n.is_empty() {
            names.entry(id).or_insert(n);
        }
    }

    for a in accounts.values_mut() {
        if let Some(p) = profiles.get(&format!("8:orgid:{}", a.user_id)) {
            a.name = opt(p.display_name.clone());
            a.email = p.email.clone().or_else(|| p.upn.clone());
        }
    }

    let messages = build_messages(raw_msgs, &names);
    let conversations = build_conversations(raw_convs, &messages, &names);
    let accounts: Vec<Account> = accounts
        .into_values()
        .filter(|a| messages.iter().any(|m| m.account == a.key) || conversations.iter().any(|c| c.account == a.key))
        .collect();

    Ok(Dataset { catalog, accounts, profiles, conversations, messages, decode_errors })
}

fn build_messages(raw: Vec<RawMsg>, names: &HashMap<String, String>) -> Vec<Message> {
    // Distinct versions per (account, conversation, message id).
    let mut groups: HashMap<(String, String, String), Vec<Message>> = HashMap::new();
    for RawMsg { account, json: m, prov } in raw {
        let props = m.get("properties").cloned().unwrap_or(Json::Null);
        let content = s(&m, "content");
        let message_type = s(&m, "messageType");
        // Fingerprint of what defines a version; blank properties are dropped
        // because Teams writes absent / null / "" interchangeably.
        let norm_props: serde_json::Map<String, Json> = props
            .as_object()
            .map(|o| o.iter().filter(|(_, v)| !blank(Some(v))).map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let deletion_info = m.get("deletionInfo").filter(|v| !blank(Some(v)));
        let fp_src =
            serde_json::json!([s(&m, "id"), s(&m, "version"), content, message_type, norm_props, deletion_info]);
        let fingerprint = hex::encode(Sha256::digest(fp_src.to_string().as_bytes()));
        let conversation_id = s(&m, "conversationId");
        let id = s(&m, "id");
        let gkey = (account.clone(), conversation_id.clone(), id.clone());
        let group = groups.entry(gkey).or_default();
        if let Some(existing) = group.iter_mut().find(|x| x.fingerprint == fingerprint) {
            existing.occurrences += 1;
            if (status_rank(prov.status), prov.seq) > (status_rank(existing.prov.status), existing.prov.seq) {
                existing.prov = prov;
            }
            continue;
        }

        let sender_mri = s(&m, "creator");
        let sender_name = [s(&m, "imDisplayName"), s(&m, "fromDisplayNameInToken")]
            .into_iter()
            .find(|x| !x.is_empty())
            .or_else(|| names.get(&sender_mri).cloned())
            .unwrap_or_else(|| if sender_mri.starts_with("19:") { "System".into() } else { sender_mri.clone() });

        let reactions = as_json(props.get("emotions").unwrap_or(&Json::Null))
            .as_array()
            .map(|list| {
                list.iter()
                    .flat_map(|e| {
                        let emoji = s(e, "key");
                        as_json(e.get("users").unwrap_or(&Json::Null))
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .map(move |u| (emoji.clone(), u))
                    })
                    .map(|(emoji, u)| {
                        let mri = s(&u, "mri");
                        Reaction {
                            name: names.get(&mri).cloned().unwrap_or_else(|| mri.clone()),
                            emoji,
                            mri,
                            time: as_ms(u.get("time")),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mentions = as_json(props.get("mentions").unwrap_or(&Json::Null))
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|x| {
                        let n = s(x, "displayName");
                        if n.is_empty() { names.get(&s(x, "mri")).cloned().unwrap_or_else(|| s(x, "mri")) } else { n }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let files: Vec<FileRef> = as_json(props.get("files").unwrap_or(&Json::Null))
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|f| {
                        let fi = f.get("fileInfo").cloned().unwrap_or(Json::Null);
                        let name = [s(f, "fileName"), s(f, "title"), s(&fi, "fileName"), s(f, "name")]
                            .into_iter()
                            .find(|x| !x.is_empty())
                            .unwrap_or_default();
                        let url = [s(&fi, "fileUrl"), s(f, "objectUrl"), s(&fi, "shareUrl"), s(f, "fileUrl")]
                            .into_iter()
                            .find(|x| !x.is_empty())
                            .unwrap_or_default();
                        FileRef { name, url }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let deleted = as_ms(props.get("deletetime")).or_else(|| as_ms(props.get("hardDeleteTime")));
        let deleted_by = opt(s(&props, "deletedBy")).map(|d| names.get(&d).cloned().unwrap_or(d));
        let mut text = readable_text(&message_type, &content, &props, names);
        if text.is_empty() && !files.is_empty() {
            text = files.iter().map(|f| format!("[file] {}", f.name)).collect::<Vec<_>>().join("\n");
        }
        if text.is_empty() && deleted.is_some() {
            text = "[message deleted]".into();
        }

        group.push(Message {
            account,
            conversation_id,
            version: s(&m, "version"),
            client_message_id: s(&m, "clientMessageId"),
            reply_chain_id: s(&m, "parentMessageId"),
            sequence_id: m.get("sequenceId").and_then(|v| v.as_f64()).map(|v| v as i64),
            arrival: as_ms(m.get("originalArrivalTime")),
            client_arrival: as_ms(m.get("clientArrivalTime")),
            compose_time: opt(s(&props, "composetime")),
            from_me: m.get("isSentByCurrentUser").and_then(|v| v.as_bool()).unwrap_or(false),
            content_type: s(&m, "contentType"),
            edited: as_ms(props.get("edittime")),
            deleted,
            deleted_by,
            importance: s(&props, "importance"),
            subject: s(&props, "subject"),
            variant: Variant::Latest,
            versions_known: 1,
            changes: vec![],
            substantive: false,
            occurrences: 1,
            fingerprint,
            raw: m,
            id,
            sender_mri,
            sender_name,
            message_type,
            content,
            text,
            reactions,
            mentions,
            files,
            prov,
        });
    }

    let mut out = Vec::new();
    for (_, mut versions) in groups {
        let ver = |m: &Message| m.version.parse::<u64>().unwrap_or(0);
        versions.sort_by(|a, b| {
            ver(a)
                .cmp(&ver(b))
                .then(status_rank(a.prov.status).cmp(&status_rank(b.prov.status)))
                .then(a.prov.seq.cmp(&b.prov.seq))
        });
        let n = versions.len();
        let latest = versions[n - 1].clone();
        for (i, mut m) in versions.into_iter().enumerate() {
            m.versions_known = n;
            if i + 1 < n {
                m.variant = Variant::Earlier;
                m.changes = changes_between(&m, &latest);
                m.substantive = m
                    .changes
                    .iter()
                    .any(|c| SUBSTANTIVE.iter().any(|k| c == k || c.strip_prefix("properties.") == Some(k)));
            }
            out.push(m);
        }
    }
    out.sort_by(|a, b| {
        (&a.account, &a.conversation_id)
            .cmp(&(&b.account, &b.conversation_id))
            .then(a.arrival.unwrap_or(0.0).total_cmp(&b.arrival.unwrap_or(0.0)))
            .then(a.id.cmp(&b.id))
            // current version first, then earlier versions, newest first
            .then((a.variant == Variant::Earlier).cmp(&(b.variant == Variant::Earlier)))
            .then(b.version.cmp(&a.version))
    });
    out
}

const SUBSTANTIVE: &[&str] = &[
    "content",
    "messageType",
    "edittime",
    "deletetime",
    "deletedBy",
    "systemdelete",
    "hardDeleteTime",
    "emotions",
    "files",
    "mentions",
];

/// Treat absent, null and "" as the same value (Teams writes them inconsistently).
fn blank(v: Option<&Json>) -> bool {
    matches!(v, None | Some(Json::Null)) || v.and_then(|x| x.as_str()) == Some("")
}

fn changes_between(a: &Message, b: &Message) -> Vec<String> {
    let mut out = Vec::new();
    if a.content != b.content {
        out.push("content".into());
    }
    if a.message_type != b.message_type {
        out.push("messageType".into());
    }
    if a.version != b.version {
        out.push("version".into());
    }
    let empty = serde_json::Map::new();
    let pa = a.raw.get("properties").and_then(|p| p.as_object()).unwrap_or(&empty);
    let pb = b.raw.get("properties").and_then(|p| p.as_object()).unwrap_or(&empty);
    let keys: std::collections::BTreeSet<&String> = pa.keys().chain(pb.keys()).collect();
    for k in keys {
        let (x, y) = (pa.get(k), pb.get(k));
        if blank(x) && blank(y) {
            continue;
        }
        if x != y {
            out.push(format!("properties.{k}"));
        }
    }
    let (da, db) = (a.raw.get("deletionInfo"), b.raw.get("deletionInfo"));
    if !(blank(da) && blank(db)) && da != db {
        out.push("deletionInfo".into());
    }
    out
}

fn build_conversations(
    raw: Vec<(String, Json, Provenance)>,
    messages: &[Message],
    names: &HashMap<String, String>,
) -> Vec<Conversation> {
    let mut best: HashMap<(String, String), (Json, Provenance)> = HashMap::new();
    for (account, v, prov) in raw {
        let key = (account, s(&v, "id"));
        let ver = |j: &Json| j.get("version").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let replace = match best.get(&key) {
            None => true,
            Some((old, op)) => {
                (status_rank(prov.status), ver(&v), prov.seq) > (status_rank(op.status), ver(old), op.seq)
            }
        };
        if replace {
            best.insert(key, (v, prov));
        }
    }

    let mut counts: HashMap<(&str, &str), (usize, f64)> = HashMap::new();
    for m in messages.iter().filter(|m| m.variant == Variant::Latest) {
        let e = counts.entry((&m.account, &m.conversation_id)).or_insert((0, 0.0));
        e.0 += 1;
        e.1 = e.1.max(m.arrival.unwrap_or(0.0));
    }

    let mut out: Vec<Conversation> = Vec::new();
    for ((account, id), (v, prov)) in best {
        let tp = v.get("threadProperties").cloned().unwrap_or(Json::Null);
        let members: Vec<Member> = v
            .get("members")
            .and_then(|m| m.as_array())
            .into_iter()
            .flatten()
            .map(|m| {
                let mri = s(m, "id");
                let n = s(m, "friendlyName");
                Member {
                    name: if n.is_empty() { names.get(&mri).cloned().unwrap_or_default() } else { n },
                    role: s(m, "role"),
                    mri,
                }
            })
            .collect();
        let chat_title = v.get("chatTitle").cloned().unwrap_or(Json::Null);
        let title =
            [s(&tp, "topic"), s(&tp, "spaceThreadTopic"), s(&chat_title, "longTitle"), s(&chat_title, "shortTitle")]
                .into_iter()
                .find(|x| !x.is_empty())
                .unwrap_or_else(|| {
                    let n: Vec<&str> = members.iter().map(|m| m.name.as_str()).filter(|n| !n.is_empty()).collect();
                    if n.is_empty() { id.clone() } else { n.join(", ") }
                });
        let (count, last) = counts.get(&(account.as_str(), id.as_str())).copied().unwrap_or((0, 0.0));
        let kind = [s(&v, "type"), s(&tp, "threadType")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
        out.push(Conversation {
            created: as_ms(tp.get("createdat")),
            last_activity: Some(last).filter(|x| *x > 0.0).or_else(|| as_ms(v.get("lastMessageTimeUtc"))),
            cache_purges: v.get("cacheDeleteHistory").and_then(|x| x.as_array()).cloned().unwrap_or_default(),
            message_count: count,
            has_metadata: true,
            raw: v,
            prov: Some(prov),
            account,
            id,
            kind,
            title,
            members,
        });
    }

    // Conversations only known from their messages.
    for ((account, id), (count, last)) in &counts {
        if !out.iter().any(|c| c.account == *account && c.id == *id) {
            out.push(Conversation {
                account: account.to_string(),
                id: id.to_string(),
                kind: String::new(),
                title: id.to_string(),
                created: None,
                last_activity: Some(*last).filter(|x| *x > 0.0),
                members: vec![],
                cache_purges: vec![],
                message_count: *count,
                has_metadata: false,
                raw: Json::Null,
                prov: None,
            });
        }
    }
    out.sort_by(|a, b| {
        a.account.cmp(&b.account).then(b.last_activity.unwrap_or(0.0).total_cmp(&a.last_activity.unwrap_or(0.0)))
    });
    out
}
