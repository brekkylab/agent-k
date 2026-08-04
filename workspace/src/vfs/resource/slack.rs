//! The Slack mount: a Slack workspace as a live directory tree.
//!
//! Unlike every other provider here, Slack has **no hierarchy to mirror**. S3 has
//! keys, Notion has a page tree, Drive has folders; Slack has conversations and a
//! message stream. So this tree is *synthesized* along the time axis — channel,
//! then date, then that day's messages — and the layout is chosen for what it
//! costs in API calls rather than for what Slack's data model looks like.
//!
//! That cost is the design constraint, and how hard it bites depends on how this
//! app is distributed. Slack's own tiers put `conversations.history` at Tier 3
//! (50+/min), but since 2025-05-29 an app that is **commercially distributed and
//! not Marketplace-approved** gets `conversations.history` and
//! `conversations.replies` at **1 request/minute, 15 objects per request**;
//! internal customer-built apps keep their existing limits
//! (<https://docs.slack.dev/apis/web-api/rate-limits>, plus the 2025-05-29 and
//! 2025-06-03 changelog entries). Measured from an internal app, 100 consecutive
//! `conversations.history` calls went through untouched at ~0.33s each — so the
//! severe tier is what a deployed agent-k faces, not what a dev workspace shows.
//! Two consequences shape the tree either way:
//!
//! - **Descending one day costs one call.** A single `conversations.history`
//!   window fills `chat.jsonl`'s bytes, the `threads/` listing and the `files/`
//!   listing together (see [`SlackResource::day`]), so walking into a date
//!   directory and listing all three of its children is one request, not four.
//! - **A thread is its own directory, shaped like a day.** `conversations.history`
//!   returns thread *roots* only; replies need one `conversations.replies` per
//!   thread. Inlining them would make `cat chat.jsonl` cost `1 + N` calls, spent
//!   whether or not anything reads those replies — and on the severe tier a
//!   20-thread day would take 21 minutes. So a thread sits under
//!   `threads/<root-ts>/` with the same two children a day has, filled by one call
//!   when it is entered.
//!
//! That last point is why a day and a thread share one shape ([`Scope`]): both are
//! a stretch of conversation, so both are `chat.jsonl` plus `files/`, and the tree
//! needs one explanation rather than two. It also settles where an attachment
//! posted *inside* a thread goes — invisible to `conversations.history`, it cannot
//! be in the day's `files/`, and it belongs in the thread's.
//!
//! A consequence of synthesizing rather than mirroring: a `.jsonl` here is a file
//! *this mount invents*, so Slack reports no length for it. Every one is listed at
//! 0 and sized by the cache wrapper's render-once path — which is free, because
//! entering the directory already fetched the bytes. An attachment, being a real
//! object Slack stores, is exact from the listing.
//!
//! Read-only: the mount serves history, profiles and file bytes, and nothing
//! posts.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::vfs::{
    accessor::{SlackAccessor, SlackConfig, is_read_denied},
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

/// The three virtual sections at the mount root.
const CHANNELS: &str = "channels";
const DMS: &str = "dms";
const USERS: &str = "users";

/// Conversation types behind each section. Slack lists both channel kinds from
/// one call, and both DM kinds from another.
const CHANNEL_TYPES: &str = "public_channel,private_channel";
const DM_TYPES: &str = "im,mpim";

/// The day file and its two subdirectories.
const CHAT_FILE: &str = "chat.jsonl";
const THREADS_DIR: &str = "threads";
const FILES_DIR: &str = "files";

/// Cache TTL for every listing this resource holds, matching the metadata
/// cache's own listing TTL ([`crate::vfs::cache`]).
///
/// This cache is not the freshness policy — the `CachedResource` wrapper above it
/// is. This one exists because a path has to become a Slack id, a date range, or
/// a day's messages before anything can be read, and re-deriving those per
/// operation would multiply the request count. Holding a *different* number here
/// buys nothing and costs two things: a read whose listing expired here but not
/// there re-fetches it (an extra request per file, on any traversal outrunning
/// the shorter TTL), and for the span between the two numbers `ls` answers from
/// one snapshot while reads resolve against another.
const TTL: Duration = Duration::from_secs(300);

/// Days of history a conversation exposes, newest backwards.
///
/// Slack has no "which days have messages" endpoint, so the date directories are
/// generated from the span between the conversation's `created` and its newest
/// message. A years-old channel would otherwise list thousands of date
/// directories, most of them quiet, and `ls` would be unreadable. 90 days is
/// mirage's bound and covers the recency an agent asks about; anything older is
/// reachable through search rather than by walking.
const MAX_DAYS: i64 = 90;

/// Hits one search returns. Slack ranks by relevance/recency, and a reader that
/// needs more than this wants a narrower query, not a longer list.
const SEARCH_MAX_HITS: usize = 100;

const SLACK_PROMPT: &str = "\
Slack (read-only) — the user's own Slack: the channels they are in, their DMs and
group DMs, by date. A thread is a directory shaped like a day.
  channels/<name>__<id>/<yyyy-mm-dd>/   dms/<user>__<id>/<yyyy-mm-dd>/
    chat.jsonl          that day's messages, one JSON object per line
    files/              attachments, cat for the real bytes
    threads/<root-ts>/  one thread: the same chat.jsonl + files/
  users/<name>__<id>.json   member profiles

  Read with jq, e.g. `jq -r '.user_name + \": \" + .text' chat.jsonl` — Slack's own
  message object plus `user_name` (mentions in `text` are already `@name`). Skip
  Slack's event notices with `select(.subtype == null)`.

  Costs: a day's chat.jsonl is ONE request and holds only the messages that START
  a thread; a root with replies has `reply_count`, and its replies are in
  threads/<that ts>/, one request when you enter it. Read the day, then enter only
  the threads worth expanding. A file posted inside a thread is in THAT thread's
  files/, never the day's.

  Every listing and read is a live API call, and reading history may be limited to
  one request per minute. Walk one level at a time; never recursive find or grep
  here. Names are not predictable — `ls` the parent rather than constructing a
  path. Dates go back 90 days at most, and exist for quiet days too.

  This is private material, DMs included. Read what the task needs and no more,
  and do not carry someone's messages into an output nobody asked for.";

/// One conversation as the tree sees it.
#[derive(Clone)]
struct Conv {
    /// Listing name: `<sanitized display name>__<id>`.
    vfs_name: String,
    id: String,
    /// Creation time (unix seconds) — the lower bound of the date range.
    created: i64,
}

/// One channel-day, all of it from a single `conversations.history` window.
struct Day {
    /// `chat.jsonl`: the day's top-level messages, one JSON object per line.
    chat: Arc<Vec<u8>>,
    /// Newest message ts in the day, for the day's mtime.
    newest: Option<f64>,
    /// Roots that have replies — the `threads/` listing.
    threads: Vec<ThreadRef>,
    /// Files shared that day — the `files/` listing.
    files: Vec<FileMeta>,
}

/// A thread as the day's listing knows it: just its root ts. That names the
/// directory, and everything inside it comes from reading the thread.
struct ThreadRef {
    ts: String,
}

/// One thread, from a single `conversations.replies` call.
struct Thread {
    /// The thread's JSONL: its root followed by every reply.
    jsonl: Arc<Vec<u8>>,
    /// Files attached to the replies — the thread's own `files/` listing.
    ///
    /// These are invisible to `conversations.history`, which returns roots only,
    /// so the day's `files/` cannot contain them: a file posted *into* a thread is
    /// only reachable through the thread. Hence a sibling directory per thread
    /// rather than one flat day listing — and the day listing stays one request,
    /// since this is filled by the same call that serves the thread.
    files: Vec<FileMeta>,
}

/// A file shared in a message, as a `files/` directory lists it.
#[derive(Clone)]
struct FileMeta {
    /// `<stem>__<file-id>.<ext>`.
    vfs_name: String,
    /// Byte length, which Slack reports in the message metadata — so an
    /// attachment's size is exact from the listing, with nothing downloaded.
    size: u64,
    /// `url_private_download`, which needs the mount's token as a bearer header.
    url: String,
    mtime: Option<SystemTime>,
}

/// Which conversation-day a `chat.jsonl` / `files/` belongs to: the day itself,
/// or one thread within it.
///
/// A day and a thread are the same thing at different scales — a stretch of
/// conversation with messages and attachments — so they carry the same two
/// children and this is the only thing that distinguishes them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Scope {
    id: String,
    date: String,
    /// `None` = the whole day; `Some(root ts)` = that thread.
    ts: Option<String>,
}

/// What a mount path names. Resolved from the path's segments alone (see
/// [`resolve`]) — every id the tree needs is in the names, so a deep path is
/// resolvable without walking its parents.
#[derive(Debug, PartialEq, Eq)]
enum Node {
    /// `/` — the three sections.
    Root,
    /// `/channels` or `/dms`.
    Convs { dms: bool },
    /// `/users`.
    Users,
    /// `/users/<name>__<id>.json`.
    User { id: String },
    /// `/{channels,dms}/<name>__<id>` — a conversation's date listing.
    Conv { id: String },
    /// A day (`/…/<date>`) or a thread (`/…/<date>/threads/<ts>`): either way, a
    /// directory holding `chat.jsonl` and `files/`.
    Convo(Scope),
    /// That scope's `chat.jsonl`.
    Chat(Scope),
    /// That scope's `files/`.
    Files(Scope),
    /// One file in it.
    File { scope: Scope, name: String },
    /// `/…/<date>/threads` — the day's threads, one directory each.
    Threads { id: String, date: String },
}

/// One cached value with the time it was fetched, shared so a `stat` or a read
/// borrows it instead of copying (a day's messages, a workspace's member list).
type Cached<T> = (Instant, Arc<T>);
/// A keyed cache of those, expiring at [`TTL`].
type CacheMap<K, T> = Mutex<HashMap<K, Cached<T>>>;

pub struct SlackResource {
    accessor: SlackAccessor,
    /// Section (`channels`/`dms`) → its conversations.
    convs: CacheMap<String, Vec<Conv>>,
    /// The workspace's members, for DM names and profile files.
    users: Mutex<Option<Cached<Vec<Value>>>>,
    /// Conversation id → its date directories (newest first).
    dates: CacheMap<String, Vec<String>>,
    /// (conversation id, date) → that day, fetched once for all three children.
    days: CacheMap<(String, String), Day>,
    /// (conversation id, root ts) → the thread's JSONL plus its own attachments.
    threads: CacheMap<(String, String), Thread>,
}

impl SlackResource {
    pub fn new(config: &SlackConfig) -> anyhow::Result<Self> {
        Ok(Self {
            accessor: SlackAccessor::new(config)?,
            convs: Mutex::new(HashMap::new()),
            users: Mutex::new(None),
            dates: Mutex::new(HashMap::new()),
            days: Mutex::new(HashMap::new()),
            threads: Mutex::new(HashMap::new()),
        })
    }

    /// The workspace's members. One call for the whole tree's naming needs.
    async fn users(&self) -> ResourceResult<Arc<Vec<Value>>> {
        if let Some((at, v)) = self.users.lock().await.as_ref()
            && at.elapsed() < TTL
        {
            return Ok(v.clone());
        }
        // Lock released across the await on purpose: a duplicate concurrent fetch
        // is harmless, holding a lock over a network call is not.
        let list = Arc::new(self.accessor.list_users().await.map_err(backend)?);
        *self.users.lock().await = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    /// user id → display name, for naming DMs.
    async fn user_names(&self) -> ResourceResult<HashMap<String, String>> {
        Ok(self
            .users()
            .await?
            .iter()
            .filter_map(|u| {
                let id = u.get("id").and_then(Value::as_str)?;
                Some((id.to_string(), display_name(u).to_string()))
            })
            .collect())
    }

    /// A section's conversations.
    async fn convs(&self, dms: bool) -> ResourceResult<Arc<Vec<Conv>>> {
        let section = if dms { DMS } else { CHANNELS };
        if let Some((at, v)) = self.convs.lock().await.get(section)
            && at.elapsed() < TTL
        {
            return Ok(v.clone());
        }
        let types = if dms { DM_TYPES } else { CHANNEL_TYPES };
        let raw = self
            .accessor
            .list_conversations(types)
            .await
            .map_err(backend)?;
        // A DM has no name of its own — it is named after the person on the other
        // side, so it needs the member list too.
        let mut names = if dms {
            self.user_names().await?
        } else {
            HashMap::new()
        };
        if dms {
            // `users.list` does not necessarily cover every DM partner: measured
            // against a real workspace, a DM with Slack's own `USLACK` account came
            // back from `conversations.list` while the member list held only
            // `USLACKBOT` and one human. Without this the entry falls back to the
            // raw id (`USLACK__D0BN…`), which tells a reader nothing about who it
            // is. One `users.info` per unresolved partner fixes it — normally zero
            // calls, since the member list usually covers everyone.
            let missing: Vec<String> = raw
                .iter()
                .filter_map(|c| c.get("user").and_then(Value::as_str))
                .filter(|uid| !uid.is_empty() && !names.contains_key(*uid))
                .map(String::from)
                .collect();
            for uid in missing {
                match self.accessor.user_info(&uid).await {
                    Ok(u) => {
                        names.insert(uid, display_name(&u).to_string());
                    }
                    // A deactivated or cross-org partner may not resolve; the id is
                    // a poor name but a stable one, so keep listing the DM.
                    Err(e) => tracing::debug!("slack: users.info({uid}) failed: {e}"),
                }
            }
        }
        let mut out: Vec<Conv> = Vec::with_capacity(raw.len());
        let mut used: HashMap<String, usize> = HashMap::new();
        for c in raw.iter() {
            let Some(id) = c.get("id").and_then(Value::as_str) else {
                continue;
            };
            let label = conv_label(c, &names);
            // Two conversations can sanitize to one name (a channel called
            // `a-b` and one called `a_b`), and the id disambiguates them — but
            // only if the whole entry name is unique, which it is: the id is part
            // of it. The counter guards the pathological case of a repeated id.
            let base = format!("{}__{id}", sanitize(&label));
            let n = used.entry(base.clone()).or_insert(0);
            *n += 1;
            let vfs_name = if *n == 1 {
                base
            } else {
                format!("{base}({n})")
            };
            out.push(Conv {
                vfs_name,
                id: id.to_string(),
                created: c.get("created").and_then(Value::as_i64).unwrap_or(0),
            });
        }
        let out = Arc::new(out);
        self.convs
            .lock()
            .await
            .insert(section.to_string(), (Instant::now(), out.clone()));
        Ok(out)
    }

    /// A conversation's `created`, from whichever section listing holds it, or
    /// `conversations.info` when neither does (a deep path resolved cold — the id
    /// came out of the path, so there is nothing to walk).
    async fn created_of(&self, id: &str) -> ResourceResult<i64> {
        for dms in [false, true] {
            let cached = self
                .convs
                .lock()
                .await
                .get(if dms { DMS } else { CHANNELS })
                .filter(|(at, _)| at.elapsed() < TTL)
                .map(|(_, v)| v.clone());
            if let Some(list) = cached
                && let Some(c) = list.iter().find(|c| c.id == id)
            {
                return Ok(c.created);
            }
        }
        let info = self.accessor.conversation_info(id).await.map_err(backend)?;
        Ok(info.get("created").and_then(Value::as_i64).unwrap_or(0))
    }

    /// A conversation's date directories, newest first.
    ///
    /// Two calls at most: one `conversations.history?limit=1` for the newest
    /// message (the range's upper bound) and, only when the conversation isn't in
    /// a cached listing, one `conversations.info` for `created`. A conversation
    /// with no messages — or one this token cannot read — has no dates.
    async fn dates(&self, id: &str) -> ResourceResult<Arc<Vec<String>>> {
        if let Some((at, v)) = self.dates.lock().await.get(id)
            && at.elapsed() < TTL
        {
            return Ok(v.clone());
        }
        let latest = match self.accessor.latest_message_ts(id).await {
            Ok(t) => t,
            // The token can't read this conversation: an empty date list, not a
            // broken tree.
            Err(e) if is_read_denied(&e) => {
                tracing::debug!("slack: history denied for {id} ({e}); listing it empty");
                None
            }
            Err(e) => return Err(backend(e)),
        };
        let dates = match latest {
            Some(newest) => {
                let created = self.created_of(id).await?;
                date_range(newest, created)
            }
            None => Vec::new(),
        };
        let dates = Arc::new(dates);
        self.dates
            .lock()
            .await
            .insert(id.to_string(), (Instant::now(), dates.clone()));
        Ok(dates)
    }

    /// One conversation-day: `chat.jsonl`'s bytes plus the `threads/` and
    /// `files/` listings, from a single `conversations.history` window.
    ///
    /// This is the primitive the layout is built around — descending a date
    /// directory and listing all three of its children costs one request.
    async fn day(&self, id: &str, date: &str) -> ResourceResult<Arc<Day>> {
        let key = (id.to_string(), date.to_string());
        if let Some((at, v)) = self.days.lock().await.get(&key)
            && at.elapsed() < TTL
        {
            return Ok(v.clone());
        }
        let (oldest, next) = day_bounds(date).ok_or(ResourceError::NotFound)?;
        let roots = match self
            .accessor
            .conversation_history(id, &fmt_ts(oldest), &fmt_ts(next))
            .await
        {
            Ok(m) => m,
            Err(e) if is_read_denied(&e) => {
                tracing::debug!("slack: history denied for {id}/{date} ({e}); serving it empty");
                Vec::new()
            }
            Err(e) => return Err(backend(e)),
        };
        // Names for the ids in each message (see `message_line`). Cached for the
        // whole mount, so this costs no request of its own; a failure to fetch
        // them leaves ids as they are rather than failing the day.
        let names = self.user_names().await.unwrap_or_default();

        let mut chat = Vec::new();
        let mut threads = Vec::new();
        let mut files = Vec::new();
        let mut newest: Option<f64> = None;
        for m in &roots {
            let ts = ts_of(m);
            // Slack's window is inclusive at both ends, so a message landing
            // exactly at the next midnight comes back for both days. Asking for
            // one tick less would instead drop anything in that tick, so the
            // window stays wide and the far edge is excluded here.
            if ts >= next as f64 {
                continue;
            }
            newest = Some(newest.map_or(ts, |n: f64| n.max(ts)));
            chat.extend_from_slice(&message_line(m, &names));
            // Only a root with replies gets a thread file; an empty `threads/`
            // then truthfully means no discussion started that day.
            if m.get("reply_count").and_then(Value::as_u64).unwrap_or(0) > 0
                && let Some(t) = m.get("ts").and_then(Value::as_str)
            {
                threads.push(ThreadRef { ts: t.to_string() });
            }
            for f in m
                .get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(meta) = file_meta(f) {
                    files.push(meta);
                }
            }
        }
        // Two uploads can share a filename within one day; the file id is in the
        // name, so entries stay distinct, and this only guards a repeated id.
        dedup_names(&mut files, |f| &mut f.vfs_name);

        let day = Arc::new(Day {
            chat: Arc::new(chat),
            newest,
            threads,
            files,
        });
        self.days
            .lock()
            .await
            .insert(key, (Instant::now(), day.clone()));
        Ok(day)
    }

    /// One thread: its JSONL plus the attachments its *replies* carry. One
    /// request, paid when either is read — the same call answers both, so opening
    /// a thread's `files/` costs nothing beyond reading the thread.
    async fn thread(&self, id: &str, ts: &str) -> ResourceResult<Arc<Thread>> {
        let key = (id.to_string(), ts.to_string());
        if let Some((at, v)) = self.threads.lock().await.get(&key)
            && at.elapsed() < TTL
        {
            return Ok(v.clone());
        }
        let msgs = match self.accessor.conversation_replies(id, ts).await {
            Ok(m) => m,
            Err(e) if is_read_denied(&e) => {
                tracing::debug!("slack: replies denied for {id}/{ts} ({e}); serving it empty");
                Vec::new()
            }
            Err(e) => return Err(backend(e)),
        };
        let names = self.user_names().await.unwrap_or_default();
        let mut jsonl = Vec::new();
        let mut files = Vec::new();
        for m in &msgs {
            jsonl.extend_from_slice(&message_line(m, &names));
            // Skip the root's own attachments: the day's `files/` already lists
            // those (it came from `conversations.history`, which returns roots),
            // and listing them twice would make the same file look like two.
            if m.get("ts").and_then(Value::as_str) == Some(ts) {
                continue;
            }
            for f in m
                .get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(meta) = file_meta(f) {
                    files.push(meta);
                }
            }
        }
        dedup_names(&mut files, |f| &mut f.vfs_name);
        let thread = Arc::new(Thread {
            jsonl: Arc::new(jsonl),
            files,
        });
        self.threads
            .lock()
            .await
            .insert(key, (Instant::now(), thread.clone()));
        Ok(thread)
    }

    /// The profile JSON one `users/<name>__<id>.json` serves.
    async fn user_profile(&self, id: &str) -> ResourceResult<Vec<u8>> {
        let users = self.users().await?;
        let u = users
            .iter()
            .find(|u| u.get("id").and_then(Value::as_str) == Some(id))
            .ok_or(ResourceError::NotFound)?;
        Ok(serde_json::to_vec_pretty(u)?)
    }

    /// A thread the day actually listed. The `ts` indexes a request, so a path
    /// naming an arbitrary one must not become a `conversations.replies` call —
    /// this is what ties it back to the day's own roots.
    async fn listed_thread(&self, id: &str, date: &str, ts: &str) -> ResourceResult<Arc<Thread>> {
        if !self.day(id, date).await?.threads.iter().any(|t| t.ts == ts) {
            return Err(ResourceError::NotFound);
        }
        self.thread(id, ts).await
    }

    /// One scope's `chat.jsonl` bytes and `files/` entries: the day's, or one
    /// thread's. Both are filled by a single request (`conversations.history` for
    /// a day, `conversations.replies` for a thread), which is what makes listing
    /// either scope's two children cost nothing extra.
    async fn contents(&self, s: &Scope) -> ResourceResult<(Arc<Vec<u8>>, Vec<FileMeta>)> {
        match &s.ts {
            None => {
                let day = self.day(&s.id, &s.date).await?;
                Ok((day.chat.clone(), day.files.clone()))
            }
            Some(ts) => {
                let t = self.listed_thread(&s.id, &s.date, ts).await?;
                Ok((t.jsonl.clone(), t.files.clone()))
            }
        }
    }

    /// Whether the scope exists at all, without producing its contents. A day
    /// exists if it is in the conversation's date range; a thread if the day
    /// listed it. Both answers come from listings a walk already fetched, so a
    /// `stat` of a made-up path costs no request of its own.
    async fn scope_exists(&self, s: &Scope) -> ResourceResult<bool> {
        if !self.dates(&s.id).await?.contains(&s.date) {
            return Ok(false);
        }
        Ok(match &s.ts {
            None => true,
            Some(ts) => self
                .day(&s.id, &s.date)
                .await?
                .threads
                .iter()
                .any(|t| &t.ts == ts),
        })
    }

    /// The scope's mtime: its newest message, falling back to the day's midnight
    /// (or the thread's own start).
    async fn scope_mtime(&self, s: &Scope) -> Option<SystemTime> {
        match &s.ts {
            Some(ts) => ts.parse::<f64>().ok().and_then(ts_time),
            None => {
                let day = self.day(&s.id, &s.date).await.ok()?;
                day.newest.and_then(ts_time).or_else(|| date_mtime(&s.date))
            }
        }
    }

    /// Fetch an attachment's bytes, applying `range` ourselves when the server
    /// ignored the header (a 200 to a ranged request carries the whole body).
    async fn download(&self, f: &FileMeta, range: Option<Range<u64>>) -> ResourceResult<Vec<u8>> {
        let (bytes, served_range) = self
            .accessor
            .download_file(&f.url, range.clone())
            .await
            .map_err(backend)?;
        Ok(if served_range {
            bytes
        } else {
            slice(&bytes, &range)
        })
    }

    /// The `files/` entry named `name` in that scope, if it exists.
    async fn file_of(&self, s: &Scope, name: &str) -> ResourceResult<FileMeta> {
        self.contents(s)
            .await?
            .1
            .iter()
            .find(|f| f.vfs_name == name)
            .cloned()
            .ok_or(ResourceError::NotFound)
    }
}

#[async_trait]
impl Resource for SlackResource {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        match resolve(path).ok_or(ResourceError::NotFound)? {
            Node::Chat(s) => Ok(slice(&self.contents(&s).await?.0, &range)),
            Node::File { scope, name } => {
                let f = self.file_of(&scope, &name).await?;
                self.download(&f, range).await
            }
            Node::User { id } => Ok(slice(&self.user_profile(&id).await?, &range)),
            // Directories have no bytes.
            _ => Err(ResourceError::NotFound),
        }
    }

    async fn write_bytes(&self, _path: &MountPath, _data: Vec<u8>) -> ResourceResult<()> {
        Err(ResourceError::Unsupported)
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        match resolve(path).ok_or(ResourceError::NotFound)? {
            Node::Root => Ok([CHANNELS, DMS, USERS]
                .iter()
                .map(|s| dir(s, None))
                .collect()),
            Node::Convs { dms } => Ok(self
                .convs(dms)
                .await?
                .iter()
                .map(|c| dir(&c.vfs_name, epoch_secs(c.created)))
                .collect()),
            Node::Users => Ok(self
                .users()
                .await?
                .iter()
                .filter_map(|u| {
                    let id = u.get("id").and_then(Value::as_str)?;
                    // Rendered from the member list already in hand, so the
                    // wrapper's eager sizing of these costs no request.
                    Some(file(&user_filename(u, id), 0, None))
                })
                .collect()),
            Node::Conv { id } => Ok(self
                .dates(&id)
                .await?
                .iter()
                .map(|d| dir(d, date_mtime(d)))
                .collect()),
            // A day and a thread list the same two children. `threads/` only
            // exists on a day — Slack has no nested threads.
            Node::Convo(s) => {
                let mtime = self.scope_mtime(&s).await;
                // Listed at 0 so the wrapper resolves the real length: this call
                // already fetched the bytes, so that costs nothing.
                let mut out = vec![file(CHAT_FILE, 0, mtime), dir(FILES_DIR, mtime)];
                if s.ts.is_none() {
                    out.push(dir(THREADS_DIR, mtime));
                }
                Ok(out)
            }
            // One directory per thread, named by its root ts — the same shape a
            // date directory has, so a thread reads exactly like a day.
            Node::Threads { id, date } => Ok(self
                .day(&id, &date)
                .await?
                .threads
                .iter()
                .map(|t| dir(&t.ts, t.ts.parse::<f64>().ok().and_then(ts_time)))
                .collect()),
            Node::Files(s) => Ok(self
                .contents(&s)
                .await?
                .1
                .iter()
                .map(|f| file(&f.vfs_name, f.size, f.mtime))
                .collect()),
            // Files aren't directories.
            Node::Chat(_) | Node::File { .. } | Node::User { .. } => Err(ResourceError::NotFound),
        }
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        let node = resolve(path).ok_or(ResourceError::NotFound)?;
        match node {
            Node::Root | Node::Convs { .. } | Node::Users => Ok(stat_dir(None)),
            Node::Conv { id } => {
                // A conversation must exist to be a directory; the section
                // listing is cached, so this is normally free.
                let c = self.conv_exists(&id).await?;
                Ok(stat_dir(epoch_secs(c)))
            }
            Node::Threads { id, date } => {
                // A date directory exists iff it is in the conversation's range —
                // which is what `readdir` of the conversation lists, so a `stat`
                // after an `ls` is served from that cached range.
                if !self.dates(&id).await?.contains(&date) {
                    return Err(ResourceError::NotFound);
                }
                Ok(stat_dir(date_mtime(&date)))
            }
            // A day or a thread directory, and their `files/`: existence comes
            // from listings a walk already fetched, not from producing contents.
            Node::Convo(s) | Node::Files(s) => {
                if !self.scope_exists(&s).await? {
                    return Err(ResourceError::NotFound);
                }
                Ok(stat_dir(self.scope_mtime(&s).await))
            }
            Node::Chat(s) => {
                let (bytes, _) = self.contents(&s).await?;
                Ok(FileStat {
                    kind: FileKind::File,
                    size: bytes.len() as u64,
                    mtime: self.scope_mtime(&s).await,
                    ..Default::default()
                })
            }
            Node::File { scope, name } => {
                let f = self.file_of(&scope, &name).await?;
                Ok(FileStat {
                    kind: FileKind::File,
                    size: f.size,
                    mtime: f.mtime,
                    ..Default::default()
                })
            }
            Node::User { id } => {
                let bytes = self.user_profile(&id).await?;
                Ok(FileStat {
                    kind: FileKind::File,
                    size: bytes.len() as u64,
                    ..Default::default()
                })
            }
        }
    }

    /// Message search over Slack's own index, designed to hang off the (not yet
    /// wired) `.cmd/` control path. Dormant today: nothing routes a write on
    /// `.cmd/<name>` here, so the mount prompt does not advertise it. Kept rather
    /// than deferred because it is the one thing reading the tree cannot do —
    /// Slack indexed the text inside uploaded files, and only search reaches it.
    ///
    /// Needs a user token; a bot-token-only mount reports that instead.
    async fn command(&self, name: &str, body: &[u8]) -> ResourceResult<Vec<u8>> {
        if name != "search" {
            return Err(ResourceError::Backend(anyhow::anyhow!(
                "unknown slack command: {name}"
            )));
        }
        let query = std::str::from_utf8(body)
            .map_err(|e| ResourceError::Backend(anyhow::anyhow!("search: body not utf-8: {e}")))?
            .trim();
        if query.is_empty() {
            // An empty query would otherwise return the whole workspace.
            return Err(ResourceError::Backend(anyhow::anyhow!(
                "search: empty query"
            )));
        }
        let v = self
            .accessor
            .search_messages(query, SEARCH_MAX_HITS)
            .await
            .map_err(backend)?;
        Ok(serde_json::to_vec(&v)?)
    }

    fn prompt(&self) -> &str {
        SLACK_PROMPT
    }
}

impl SlackResource {
    /// `created` of the conversation `id`, erroring [`ResourceError::NotFound`]
    /// when no section lists it. Unlike [`Self::created_of`], this does not fall
    /// back to `conversations.info` — a `stat` of a made-up name must not become
    /// a request, and a real conversation is in one of the two listings.
    async fn conv_exists(&self, id: &str) -> ResourceResult<i64> {
        for dms in [false, true] {
            if let Some(c) = self.convs(dms).await?.iter().find(|c| c.id == id) {
                return Ok(c.created);
            }
        }
        Err(ResourceError::NotFound)
    }
}

// ---- path resolution ------------------------------------------------------

/// Mount-relative path segments (`/channels/x__C1/2026-08-03/chat.jsonl` → 4).
fn segments(path: &MountPath) -> Vec<&str> {
    path.as_str()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect()
}

/// What a path names, from its segments alone.
///
/// Every id is in the entry names (`<name>__<id>`), so this needs no cached
/// parent listing and no request: a path handed to us cold — the WebDAV layer and
/// the guest FUSE client both do that after a restart — resolves the same as one
/// reached by walking. `None` = the path cannot name a node in this tree.
fn resolve(path: &MountPath) -> Option<Node> {
    let seg = segments(path);
    let conv_id = |s: &str| id_from_name(s).filter(|id| valid_slack_id(id));
    match seg.as_slice() {
        [] => Some(Node::Root),
        [s] if *s == CHANNELS => Some(Node::Convs { dms: false }),
        [s] if *s == DMS => Some(Node::Convs { dms: true }),
        [s] if *s == USERS => Some(Node::Users),
        [s, name] if *s == USERS => {
            let stem = name.strip_suffix(".json")?;
            Some(Node::User { id: conv_id(stem)? })
        }
        [s, conv] if *s == CHANNELS || *s == DMS => Some(Node::Conv { id: conv_id(conv)? }),
        // A day, then whatever is inside it. A thread re-enters the same tail via
        // `threads/<ts>`, which is why the two have identical children.
        [s, conv, date, rest @ ..] if (*s == CHANNELS || *s == DMS) && is_date(date) => {
            let scope = Scope {
                id: conv_id(conv)?,
                date: date.to_string(),
                ts: None,
            };
            match rest {
                // The day's own `threads/` listing has no counterpart inside a
                // thread (Slack has no nested threads), so it is handled here
                // rather than in `within`.
                [d] if *d == THREADS_DIR => Some(Node::Threads {
                    id: scope.id,
                    date: scope.date,
                }),
                [d, ts, rest @ ..] if *d == THREADS_DIR => within(
                    Scope {
                        ts: Some(valid_ts(ts)?),
                        ..scope
                    },
                    rest,
                ),
                _ => within(scope, rest),
            }
        }
        _ => None,
    }
}

/// The part of a path below a day or a thread — both hold exactly `chat.jsonl`
/// and `files/`, so one function resolves both.
fn within(scope: Scope, rest: &[&str]) -> Option<Node> {
    match rest {
        [] => Some(Node::Convo(scope)),
        [leaf] if *leaf == CHAT_FILE => Some(Node::Chat(scope)),
        [leaf] if *leaf == FILES_DIR => Some(Node::Files(scope)),
        [d, name] if *d == FILES_DIR => Some(Node::File {
            scope,
            name: (*name).to_string(),
        }),
        _ => None,
    }
}

/// A Slack message timestamp (`1785737875.341929`) — digits and one dot. Checked
/// because a `ts` becomes a `conversations.replies` argument, so an arbitrary
/// segment must not reach one.
fn valid_ts(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= 32
        && s.chars().all(|c| c.is_ascii_digit() || c == '.')
        && s.matches('.').count() <= 1
        && s.starts_with(|c: char| c.is_ascii_digit());
    ok.then(|| s.to_string())
}

/// The id encoded in a `<name>__<id>` entry (the part after the last `__`).
/// `None` when the name carries none, so a caller can reject it rather than
/// treat the whole name as an id.
fn id_from_name(name: &str) -> Option<String> {
    let (_, id) = name.rsplit_once("__")?;
    // A collision suffix (`…__C123(2)`) is part of the entry name, not the id.
    let id = id.split('(').next().unwrap_or(id);
    (!id.is_empty()).then(|| id.to_string())
}

/// Whether `s` looks like a Slack object id (`C0123ABC`, `U04…`): uppercase
/// alphanumerics. Checked before an id reaches a request, so a path cannot
/// smuggle a query into one.
fn valid_slack_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
}

/// Whether `s` is a `yyyy-mm-dd` date — how a date directory is told apart from
/// anything else at that level.
fn is_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

// ---- naming ---------------------------------------------------------------

/// Character cap for the display part of an entry name.
const NAME_MAX: usize = 80;
/// Byte ceiling for the display part, on top of [`NAME_MAX`].
///
/// A filename is capped at 255 **bytes** on the filesystems this deploys to
/// (ext4/XFS), and exceeding it fails the write with `ENAMETOOLONG`. macOS caps
/// at 255 *characters* instead, so a name Linux rejects works fine on a dev
/// machine; only the byte bound catches it. Leaves room for the `__<id>` tail.
const NAME_MAX_BYTES: usize = 200;

/// Sanitize a display name into one path segment: keep word chars, `-`, `.`,
/// collapse everything else (including whitespace and `/`) to `_`, squeeze
/// repeats, trim, and cap by characters *and* bytes.
fn sanitize(text: &str) -> String {
    if text.trim().is_empty() {
        return "unnamed".to_string();
    }
    let mut s = String::with_capacity(text.len());
    let mut prev_us = false;
    for ch in text.chars() {
        let keep = ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.';
        if keep {
            s.push(ch);
            prev_us = false;
        } else {
            if !prev_us {
                s.push('_');
            }
            prev_us = true;
        }
    }
    let mut out: String = s.trim_matches('_').to_string();
    if out.chars().count() > NAME_MAX {
        out = out.chars().take(NAME_MAX - 3).collect::<String>() + "...";
    }
    // Then bound the bytes: NAME_MAX counts characters, and 80 CJK or emoji
    // characters run to 240–320 bytes on their own. Cut on a char boundary so
    // multibyte text is shortened, never corrupted.
    if out.len() > NAME_MAX_BYTES {
        let keep = NAME_MAX_BYTES - 3;
        let cut = out
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i <= keep)
            .last()
            .unwrap_or(0);
        out.truncate(cut);
        out.push_str("...");
    }
    if out.is_empty() {
        return "unnamed".to_string();
    }
    out
}

/// The label a conversation is named after: a channel's own name, or for a DM
/// the person on the other side (Slack gives a DM no name of its own).
fn conv_label(c: &Value, user_names: &HashMap<String, String>) -> String {
    if let Some(n) = c
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    {
        return n.to_string();
    }
    let uid = c.get("user").and_then(Value::as_str).unwrap_or("");
    user_names
        .get(uid)
        .cloned()
        .unwrap_or_else(|| uid.to_string())
}

/// A member's display name, preferring the human-facing fields Slack fills.
fn display_name(u: &Value) -> &str {
    for key in ["display_name", "real_name"] {
        if let Some(v) = u
            .get("profile")
            .and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return v;
        }
    }
    u.get("name")
        .or_else(|| u.get("real_name"))
        .and_then(Value::as_str)
        .unwrap_or("unnamed")
}

fn user_filename(u: &Value, id: &str) -> String {
    format!("{}__{id}.json", sanitize(display_name(u)))
}

/// Serialize one message as a `.jsonl` line, with the ids a reader cannot resolve
/// on its own turned into names.
///
/// Slack puts only ids in a message: the author is `"user": "U0BM…"` and a mention
/// in the body is `<@U0BM…>`. Both are unreadable as they stand — resolving them
/// would mean the reader cross-referencing `users/` per line, and the mention text
/// would stay opaque even then. The member list is already in hand (it names the
/// DMs), so this fills them in at zero extra cost:
///
/// - `user_name` is **added** alongside `user`. Added rather than substituted
///   because the id is the stable identity — a display name can change or repeat,
///   and `users/<name>__<id>.json` is still found by id.
/// - `<@U0BM…>` in `text` becomes `@name`, since that is the part a person reads.
///
/// An id the member list doesn't cover is left exactly as it was: a wrong name is
/// worse than a raw id.
fn message_line(m: &Value, names: &HashMap<String, String>) -> Vec<u8> {
    let mut m = m.clone();
    if let Some(obj) = m.as_object_mut() {
        if let Some(name) = obj
            .get("user")
            .and_then(Value::as_str)
            .and_then(|u| names.get(u))
        {
            obj.insert("user_name".into(), Value::String(name.clone()));
        }
        if let Some(text) = obj.get("text").and_then(Value::as_str) {
            let resolved = resolve_mentions(text, names);
            obj.insert("text".into(), Value::String(resolved));
        }
    }
    let mut line = serde_json::to_vec(&m).unwrap_or_default();
    line.push(b'\n');
    line
}

/// Replace `<@U0BM…>` mentions with `@name`. Slack's own form also allows a label
/// (`<@U0BM…|name>`), which is handled by taking everything up to `|` or `>`.
/// Anything not resolvable is left untouched.
fn resolve_mentions(text: &str, names: &HashMap<String, String>) -> String {
    // Cheap bail-out: most messages carry no mention at all.
    if !text.contains("<@") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('>') else {
            // Unterminated: the rest is literal text.
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let id = inner.split('|').next().unwrap_or(inner);
        match names.get(id) {
            Some(name) => out.push_str(&format!("@{name}")),
            // Unknown id: keep the original token rather than invent a name.
            None => out.push_str(&rest[start..start + 2 + end + 1]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A `files/` entry from a message's `files[]` element. `None` when Slack gave no
/// id or no download URL — a tombstone for a deleted file, or one whose bytes
/// this token may not fetch, either way nothing to serve.
fn file_meta(f: &Value) -> Option<FileMeta> {
    let id = f
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let url = f
        .get("url_private_download")
        .or_else(|| f.get("url_private"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let raw = f
        .get("name")
        .or_else(|| f.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("file");
    Some(FileMeta {
        vfs_name: file_blob_name(raw, id),
        size: f.get("size").and_then(Value::as_u64).unwrap_or(0),
        url: url.to_string(),
        mtime: f
            .get("timestamp")
            .and_then(Value::as_i64)
            .and_then(epoch_secs),
    })
}

/// `<stem>__<file-id>.<ext>` — the extension is preserved so tools that dispatch
/// on it (`docling`, `file`, a `*.pdf` glob) still work on a downloaded file.
fn file_blob_name(raw: &str, id: &str) -> String {
    match raw.rsplit_once('.') {
        // Only a plausible extension: a dotted stem (`v1.2 notes`) must not have
        // its tail treated as one.
        Some((stem, ext))
            if !ext.is_empty()
                && ext.len() <= 12
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            format!("{}__{id}.{}", sanitize(stem), ext.to_lowercase())
        }
        _ => format!("{}__{id}", sanitize(raw)),
    }
}

/// Number repeated names (`x__F1`, `x__F1(2)`) so one listing has no duplicates.
fn dedup_names<T>(items: &mut [T], name: impl Fn(&mut T) -> &mut String) {
    let mut used: HashMap<String, usize> = HashMap::new();
    for item in items.iter_mut() {
        let n = name(item);
        let count = used.entry(n.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            let c = *count;
            match n.rsplit_once('.') {
                // Number before the extension, so a numbered name stays inside
                // any glob that would have found the original.
                Some((stem, ext)) => *n = format!("{stem}({c}).{ext}"),
                None => n.push_str(&format!("({c})")),
            }
        }
    }
}

// ---- time -----------------------------------------------------------------

/// The dates a conversation exposes, newest first: back from the newest message
/// to `created`, at most [`MAX_DAYS`].
fn date_range(newest_ts: f64, created: i64) -> Vec<String> {
    let Some(end) = DateTime::from_timestamp(newest_ts as i64, 0) else {
        return Vec::new();
    };
    let end = end.date_naive();
    let start = DateTime::from_timestamp(created.max(0), 0)
        .map(|d| d.date_naive())
        .unwrap_or(end);
    let start = start
        .min(end)
        .max(end - chrono::Duration::days(MAX_DAYS - 1));
    let mut out = Vec::new();
    let mut d = end;
    while d >= start {
        out.push(d.format("%Y-%m-%d").to_string());
        let Some(prev) = d.pred_opt() else { break };
        d = prev;
    }
    out
}

/// `(midnight, next midnight)` of `date` as unix seconds, UTC. Both are passed to
/// Slack (whose window is inclusive at both ends) and the far edge is dropped by
/// the caller — see [`SlackResource::day`].
fn day_bounds(date: &str) -> Option<(i64, i64)> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let start = d.and_hms_opt(0, 0, 0)?.and_utc().timestamp();
    Some((start, start + 86_400))
}

/// Slack's ts format for a whole second.
fn fmt_ts(secs: i64) -> String {
    format!("{secs}.000000")
}

/// A message's `ts` as a float (0.0 when absent), for ordering and mtimes.
fn ts_of(m: &Value) -> f64 {
    m.get("ts")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn ts_time(ts: f64) -> Option<SystemTime> {
    (ts > 0.0).then(|| UNIX_EPOCH + Duration::from_secs_f64(ts))
}

fn epoch_secs(secs: i64) -> Option<SystemTime> {
    (secs > 0).then(|| UNIX_EPOCH + Duration::from_secs(secs as u64))
}

/// A date directory's mtime: its own midnight, so `ls -l` shows the day it holds
/// rather than the epoch.
fn date_mtime(date: &str) -> Option<SystemTime> {
    day_bounds(date).and_then(|(start, _)| epoch_secs(start))
}

// ---- small helpers --------------------------------------------------------

fn backend(e: anyhow::Error) -> ResourceError {
    ResourceError::Backend(e)
}

fn dir(name: &str, mtime: Option<SystemTime>) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        kind: FileKind::Dir,
        size: 0,
        mtime,
        atime: None,
        ctime: None,
        created: None,
        etag: None,
    }
}

fn file(name: &str, size: u64, mtime: Option<SystemTime>) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        kind: FileKind::File,
        size,
        mtime,
        atime: None,
        ctime: None,
        created: None,
        etag: None,
    }
}

fn stat_dir(mtime: Option<SystemTime>) -> FileStat {
    FileStat {
        kind: FileKind::Dir,
        mtime,
        ..Default::default()
    }
}

/// Slice `data` by an optional byte range (clamped to bounds); `None` = all.
fn slice(data: &[u8], range: &Option<Range<u64>>) -> Vec<u8> {
    match range {
        Some(r) => {
            let start = (r.start as usize).min(data.len());
            let end = (r.end as usize).min(data.len()).max(start);
            data[start..end].to_vec()
        }
        None => data.to_vec(),
    }
}

#[cfg(test)]
mod tests;
