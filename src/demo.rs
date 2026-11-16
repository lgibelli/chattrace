// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthetic demo dataset (fictional people, fictional tenant) used for
//! screenshots, documentation and training. Contains no real data.

use crate::idb::{Catalog, Status};
use crate::leveldb::FileStats;
use crate::teams::{Account, Conversation, Dataset, FileRef, Member, Message, Profile, Provenance, Reaction, Variant};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

const TENANT: &str = "00000000-0000-4000-8000-0000000d3e70";
const ME: &str = "8:orgid:00000000-0000-4000-8000-00000000a1e1";
const SAM: &str = "8:orgid:00000000-0000-4000-8000-00000000b0b2";
const PRIYA: &str = "8:orgid:00000000-0000-4000-8000-00000000c3c3";
const JORDAN: &str = "8:orgid:00000000-0000-4000-8000-00000000d4d4";
const ACC: &str = "demo";

/// 2026-03-09 08:30:00 UTC (a Monday).
const T0: f64 = 1_773_045_000_000.0;
const MIN: f64 = 60_000.0;
const DAY: f64 = 86_400_000.0;

fn prov(seq: u64, status: Status) -> Provenance {
    Provenance {
        database: format!("Teams:replychain-manager:react-web-client:{TENANT}:{}:en-us", &ME[8..]),
        store: "replychains".into(),
        file: if seq < 5000 { "000412.ldb".into() } else { "000418.log".into() },
        seq,
        status,
        crc_ok: true,
    }
}

fn name(mri: &str) -> &'static str {
    match mri {
        ME => "Alex Rivera",
        SAM => "Sam Okafor",
        PRIYA => "Priya Natarajan",
        JORDAN => "Jordan Lee",
        _ => "System",
    }
}

struct M<'a> {
    conv: &'a str,
    id: u64,
    at: f64,
    from: &'a str,
    text: &'a str,
}

fn base(m: M, seq: u64) -> Message {
    let content = format!("<p>{}</p>", m.text);
    let raw = json!({
        "id": m.id.to_string(),
        "conversationId": m.conv,
        "clientMessageId": format!("{}", m.id * 7 + 13),
        "originalArrivalTime": crate::teams::utc(Some(m.at)),
        "clientArrivalTime": crate::teams::utc(Some(m.at + 412.0)),
        "creator": m.from,
        "imDisplayName": name(m.from),
        "messageType": "RichText/Html",
        "contentType": "text",
        "content": content,
        "version": format!("{}", m.at as u64),
        "sequenceId": m.id % 1000,
        "isSentByCurrentUser": m.from == ME,
        "properties": {}
    });
    Message {
        account: ACC.into(),
        conversation_id: m.conv.into(),
        id: m.id.to_string(),
        version: format!("{}", m.at as u64),
        client_message_id: format!("{}", m.id * 7 + 13),
        reply_chain_id: m.id.to_string(),
        sequence_id: Some((m.id % 1000) as i64),
        arrival: Some(m.at),
        client_arrival: Some(m.at + 412.0),
        compose_time: Some(crate::teams::utc(Some(m.at - 9_000.0))),
        sender_mri: m.from.into(),
        sender_name: name(m.from).into(),
        from_me: m.from == ME,
        message_type: "RichText/Html".into(),
        content_type: "text".into(),
        content,
        text: m.text.into(),
        edited: None,
        deleted: None,
        deleted_by: None,
        reactions: vec![],
        mentions: vec![],
        files: vec![],
        importance: String::new(),
        subject: String::new(),
        variant: Variant::Latest,
        versions_known: 1,
        changes: vec![],
        substantive: false,
        occurrences: 1,
        fingerprint: format!("{:016x}", m.id.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
        raw,
        prov: prov(seq, Status::Live),
    }
}

fn react(emoji: &str, mri: &str, at: f64) -> Reaction {
    Reaction { emoji: emoji.into(), mri: mri.into(), name: name(mri).into(), time: Some(at) }
}

fn member(mri: &str, role: &str) -> Member {
    Member { mri: mri.into(), role: role.into(), name: name(mri).into() }
}

pub fn dataset() -> Dataset {
    let atlas = "19:demo-atlas-vendor-review@thread.v2";
    let sam = "19:demo-1on1-alex-sam@unq.gbl.spaces";
    let sync = "19:meeting_demo-weekly-sync@thread.v2";
    let mut msgs: Vec<Message> = Vec::new();
    let mut seq = 4100u64;
    let mut next = || {
        seq += 7;
        seq
    };

    // --- Project Atlas group chat
    let a = |id, at, from, text| M { conv: atlas, id, at, from, text };
    let mut m = base(
        a(1_773_045_000_001, T0, PRIYA, "Morning all. The three vendor proposals for Atlas are in the shared folder."),
        next(),
    );
    m.files = vec![FileRef {
        name: "Atlas vendor comparison.xlsx".into(),
        url: "https://halden.example/sites/atlas/Shared Documents/Atlas vendor comparison.xlsx".into(),
    }];
    m.raw["properties"]["files"] = json!([{"fileName": "Atlas vendor comparison.xlsx", "objectUrl": "https://halden.example/sites/atlas/Shared Documents/Atlas vendor comparison.xlsx"}]);
    msgs.push(m);
    msgs.push(base(
        a(1_773_045_000_002, T0 + 6.0 * MIN, ME, "Thanks Priya. I'll go through them before the 2 pm review."),
        next(),
    ));

    let at = T0 + 14.0 * MIN;
    let mut latest = base(
        a(1_773_045_000_003, at, SAM, "Brightline's quote is the lowest, but their delivery date is end of May."),
        next(),
    );
    latest.edited = Some(at + 3.0 * MIN);
    latest.versions_known = 2;
    latest.raw["properties"]["edittime"] = json!(format!("{}", (at + 3.0 * MIN) as u64));
    let mut earlier = base(
        a(1_773_045_000_003, at, SAM, "Brightline's quote is the lowest, but their delivery date is end of April."),
        3880,
    );
    earlier.variant = Variant::Earlier;
    earlier.versions_known = 2;
    earlier.changes = vec!["content".into(), "properties.edittime".into(), "version".into()];
    earlier.substantive = true;
    earlier.prov = prov(3880, Status::Superseded);
    latest.reactions = vec![react("like", PRIYA, at + 20.0 * MIN)];
    latest.raw["properties"]["emotions"] =
        json!([{"key": "like", "users": [{"mri": PRIYA, "time": (at + 20.0 * MIN) as u64}]}]);
    msgs.push(latest);
    msgs.push(earlier);

    let mut m =
        base(a(1_773_045_000_004, T0 + 22.0 * MIN, JORDAN, "Can we ask them to confirm the date in writing?"), next());
    m.mentions = vec![name(SAM).into()];
    m.reactions = vec![react("like", ME, T0 + 25.0 * MIN), react("heart", SAM, T0 + 31.0 * MIN)];
    msgs.push(m);

    let at = T0 + 41.0 * MIN;
    let mut del = base(a(1_773_045_000_005, at, SAM, "[message deleted]"), next());
    del.deleted = Some(at + 12.0 * MIN);
    del.deleted_by = Some(SAM.into());
    del.content = String::new();
    del.versions_known = 2;
    del.raw["content"] = json!("");
    del.raw["properties"]["deletetime"] = json!(format!("{}", (at + 12.0 * MIN) as u64));
    let mut before =
        base(a(1_773_045_000_005, at, SAM, "Honestly I'd just go with Brightline and fix the date later."), 3911);
    before.variant = Variant::Earlier;
    before.versions_known = 2;
    before.changes = vec!["content".into(), "properties.deletetime".into()];
    before.substantive = true;
    before.prov = prov(3911, Status::Superseded);
    msgs.push(del);
    msgs.push(before);

    msgs.push(base(
        a(
            1_773_045_000_006,
            T0 + DAY + 2.0 * MIN,
            PRIYA,
            "Brightline confirmed 30 May in writing. Updated the comparison sheet.",
        ),
        next(),
    ));
    msgs.push(base(
        a(
            1_773_045_000_007,
            T0 + DAY + 9.0 * MIN,
            ME,
            "Great, then I'm fine with Brightline. Let's decide on Thursday.",
        ),
        next(),
    ));

    // --- 1:1 with Sam
    let s = |id, at, from, text| M { conv: sam, id, at, from, text };
    msgs.push(base(
        s(1_773_045_100_001, T0 + 3.0 * 60.0 * MIN, SAM, "Do you have five minutes before the review?"),
        next(),
    ));
    msgs.push(base(s(1_773_045_100_002, T0 + 3.0 * 60.0 * MIN + 2.0 * MIN, ME, "Sure, calling you now."), next()));
    let mut call = base(
        s(
            1_773_045_100_003,
            T0 + 3.0 * 60.0 * MIN + 3.0 * MIN,
            SAM,
            "Call ended · 6 min 12 s · Sam Okafor, Alex Rivera",
        ),
        next(),
    );
    call.message_type = "Event/Call".into();
    call.sender_name = "System".into();
    call.sender_mri = sam.into();
    call.raw["messageType"] = json!("Event/Call");
    msgs.push(call);

    // --- Meeting chat
    let w = |id, at, from, text| M { conv: sync, id, at, from, text };
    let mut start = base(w(1_773_045_200_001, T0 - 5.0 * DAY + 60.0 * MIN, JORDAN, "Meeting started"), next());
    start.message_type = "ThreadActivity/MeetingStarted".into();
    start.raw["messageType"] = json!("ThreadActivity/MeetingStarted");
    msgs.push(start);
    msgs.push(base(
        w(
            1_773_045_200_002,
            T0 - 5.0 * DAY + 64.0 * MIN,
            PRIYA,
            "Notes from today: shortlist the three Atlas vendors, decision next week.",
        ),
        next(),
    ));
    let mut m =
        base(w(1_773_045_200_003, T0 - 5.0 * DAY + 66.0 * MIN, ME, "Thanks. I'll collect the proposals."), next());
    m.importance = "high".into();
    msgs.push(m);

    let conv =
        |id: &str, kind: &str, title: &str, created: f64, members: Vec<Member>, purges: Vec<Json>| Conversation {
            account: ACC.into(),
            id: id.into(),
            kind: kind.into(),
            title: title.into(),
            created: Some(created),
            last_activity: None,
            members,
            cache_purges: purges,
            message_count: 0,
            has_metadata: true,
            raw: json!({"id": id, "threadProperties": {"topic": title}}),
            prov: Some(Provenance {
                database: format!("Teams:conversation-manager:react-web-client:{TENANT}:{}:en-us", &ME[8..]),
                store: "conversations".into(),
                file: "000412.ldb".into(),
                seq: 2210,
                status: Status::Live,
                crc_ok: true,
            }),
        };
    let mut conversations = vec![
        conv(
            atlas,
            "Group chat",
            "Project Atlas · vendor review",
            T0 - 20.0 * DAY,
            vec![member(ME, "Admin"), member(PRIYA, "Admin"), member(SAM, "User"), member(JORDAN, "User")],
            vec![json!({"deleteTime": (T0 - 2.0 * DAY) as u64, "deletedRCsCount": 38,
                        "startOfSyncWindow": (T0 - 120.0 * DAY) as u64, "endOfSyncWindow": (T0 - 30.0 * DAY) as u64})],
        ),
        conv(sam, "1:1 chat", "Sam Okafor", T0 - 200.0 * DAY, vec![member(ME, "Admin"), member(SAM, "Admin")], vec![]),
        conv(
            sync,
            "Meeting",
            "Weekly sync",
            T0 - 5.0 * DAY,
            vec![member(ME, "User"), member(PRIYA, "User"), member(JORDAN, "Admin")],
            vec![],
        ),
    ];
    for c in &mut conversations {
        let mine: Vec<&Message> =
            msgs.iter().filter(|m| m.conversation_id == c.id && m.variant == Variant::Latest).collect();
        c.message_count = mine.len();
        c.last_activity = mine.iter().filter_map(|m| m.arrival).reduce(f64::max);
    }
    conversations.sort_by(|a, b| b.last_activity.partial_cmp(&a.last_activity).unwrap_or(std::cmp::Ordering::Equal));
    msgs.sort_by(|a, b| {
        (a.conversation_id.as_str(), a.arrival.unwrap_or(0.0) as i64, a.id.as_str(), a.variant == Variant::Earlier).cmp(
            &(
                b.conversation_id.as_str(),
                b.arrival.unwrap_or(0.0) as i64,
                b.id.as_str(),
                b.variant == Variant::Earlier,
            ),
        )
    });

    let mut profiles = BTreeMap::new();
    for (mri, email, title) in [
        (ME, "alex.rivera@halden.example", "Procurement lead"),
        (SAM, "sam.okafor@halden.example", "Engineer"),
        (PRIYA, "priya.natarajan@halden.example", "Project manager"),
        (JORDAN, "jordan.lee@halden.example", "Legal counsel"),
    ] {
        profiles.insert(
            mri.to_string(),
            Profile {
                mri: mri.into(),
                display_name: name(mri).into(),
                email: Some(email.into()),
                upn: Some(email.into()),
                job_title: Some(title.into()),
                tenant_name: Some("Halden & Co (demo)".into()),
                prov: prov(1200, Status::Live),
            },
        );
    }

    Dataset {
        catalog: Catalog {
            files: vec![
                FileStats { name: "000412.ldb".into(), records: 5120, crc_errors: 0, errors: vec![] },
                FileStats { name: "000418.log".into(), records: 311, crc_errors: 0, errors: vec![] },
            ],
            databases: BTreeMap::new(),
        },
        accounts: vec![Account {
            key: ACC.into(),
            tenant_id: TENANT.into(),
            user_id: ME[8..].into(),
            name: Some("Alex Rivera".into()),
            email: Some("alex.rivera@halden.example".into()),
            locales: vec!["en-us".into()],
        }],
        profiles,
        conversations,
        messages: msgs,
        decode_errors: vec![],
    }
}

/// Capture details for the demo report.
pub fn capture() -> Json {
    json!({
        "tool": { "name": crate::APP_NAME, "version": crate::VERSION },
        "case_reference": "DEMO-0001",
        "examiner": "Demo examiner",
        "notes": "Synthetic demo dataset: fictional people and organization, no real data.",
        "started_utc": "2026-03-12T16:05:31.204Z",
        "started_local": "2026-03-12T17:05:31.204+01:00",
        "timezone": "Europe/Rome",
        "host": { "hostname": "demo-host", "os": "macos", "os_version": "26.1", "os_user": "examiner", "teams_app_version": "25290.205" },
        "source_paths": ["/Volumes/EVIDENCE/Users/arivera/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb"],
        "source_kind": "New Teams · work/school",
        "source_is_live_local_profile": false,
        "source_volume_read_only": true,
        "teams_running_during_capture": false,
        "method": "Demo dataset.",
        "warnings": [],
        "limitations": crate::evidence::limitations(),
    })
}
