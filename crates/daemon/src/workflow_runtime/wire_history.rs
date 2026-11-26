//! Bounded wire previews and on-demand reads of the retained records.
use super::{Admission, CommandHandle, Error, Exec, Result};
use farcooler_protocol::workflow_commands::{Action, HistorySection};
use farcooler_workflow::{Archived, Item, Phase, State, digest};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

const LANDING_PREVIEW: usize = 12;
const GROUP_BYTES: usize = 80 * 1024;
const ARCHIVE_BYTES: usize = 64 * 1024;
const RUN_HISTORY_BYTES: usize = 32 * 1024;
const SEEN_BYTES: usize = 16 * 1024;
const SOURCE_SEQUENCE_BYTES: usize = 16 * 1024;
const HISTORY_CHUNK_BYTES: usize = 128 * 1024;
// Reserve room for the enclosing protobuf response and its IDs.
const GET_JSON_BYTES: usize = farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES - 256;

fn historical_group(item: &Item, joined: &BTreeSet<&str>, id: &str) -> bool {
    item.group_of.is_some() && matches!(item.phase, Phase::Done | Phase::Cancelled | Phase::Failed)
        && item.members.is_empty() && item.pending.is_none() && item.deciding.is_none()
        && item.stopping.is_none() && !joined.contains(id)
}

fn spend(budget: &mut usize, value: &impl Serialize) -> Result<bool> {
    // One extra byte accommodates the comma between adjacent entries.
    let size = serde_json::to_vec(value)?.len() + 1;
    if let Some(remaining) = budget.checked_sub(size) {
        *budget = remaining;
        return Ok(true);
    }
    Ok(false)
}

#[derive(Default)]
pub(super) struct Preview {
    pub groups_omitted: usize,
    pub archive_omitted: usize,
    pub runs_omitted: usize,
    archive_total: usize,
    groups: Vec<String>,
    dependencies: BTreeSet<String>,
}

/// Keep complete preview rows for clients that decode State/Item.
pub(super) fn preview(state: &mut State) -> Result<Preview> {
    let joined: BTreeSet<_> = state.items.values()
        .filter(|item| !matches!(item.phase, Phase::Done | Phase::Cancelled))
        .filter_map(|item| item.grouped_in.as_deref()).collect();
    let mut groups: Vec<_> = state.items.iter().filter(|(id, item)| historical_group(item, &joined, id))
        .map(|(id, item)| (item.since_ms, id.clone(), true))
        .chain(state.archive.iter()
            .filter_map(|(id, item)| Some((item.group.as_ref()?.since_ms, id.clone(), false))))
        .collect();
    groups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut preview = Preview::default();
    let mut group_budget = GROUP_BYTES;
    for (index, (_, id, live)) in groups.into_iter().enumerate() {
        // Detached terminal groups, including their mutation guards, stay within
        // the preview budget. Targeted history reads can still retrieve them.
        let keep = index < LANDING_PREVIEW && if live {
            spend(&mut group_budget, &(&id, &state.items[&id]))?
        } else {
            spend(&mut group_budget, &(&id, &state.archive[&id]))?
        };
        if keep && live {
            preview.groups.push(id);
        } else if !keep {
            if live { state.items.remove(&id); } else { state.archive.remove(&id); }
            preview.groups_omitted += 1;
        }
    }

    preview.dependencies = state.items.values().flat_map(|item| &item.dependencies).cloned().collect();
    let mut archive_budget = ARCHIVE_BYTES;
    let mut kept = BTreeMap::new();
    for (id, item) in std::mem::take(&mut state.archive).into_iter().rev() {
        preview.archive_total += usize::from(item.group.is_none());
        if item.group.is_some() || spend(&mut archive_budget, &(&id, archived_value(&id, &item)?))? {
            kept.insert(id, item);
        } else {
            preview.archive_omitted += 1;
            if item.phase == Phase::Done && preview.dependencies.contains(&id) {
                // Old clients use this phase to explain why a live dependency is still waiting.
                kept.insert(id, Archived { phase: item.phase, candidate: None, group: None,
                    start_load: None, closed: None, last_run: None, history: Vec::new() });
            }
        }
    }
    state.archive = kept;

    let mut run_budget = RUN_HISTORY_BYTES;
    for item in state.items.values_mut() {
        if !item.history.is_empty() && !spend(&mut run_budget, &item.history)? {
            preview.runs_omitted += item.history.len();
            item.history.clear();
        }
    }
    // Preserve the full schema's small deduplication sets. Older entries stay
    // durable when the set outgrows the wire preview.
    if serde_json::to_vec(&state.seen)?.len() > SEEN_BYTES { state.seen.clear(); }

    Ok(preview)
}

pub(crate) struct WorkflowRead {
    pub(crate) value: Value,
    pub(super) preview: Preview,
}

fn omitted(reply: &mut Value, field: &str, count: usize) {
    if count > 0 {
        reply[field] = (reply[field].as_u64().unwrap_or_default() + count as u64).into();
    }
}

impl WorkflowRead {
    /// Re-pin reads only the published workflow.
    pub(crate) fn keep_publication(&mut self) {
        if let Some(fields) = self.value.as_object_mut() {
            fields.retain(|key, _| matches!(key.as_str(), "schema" | "configuration" | "context"));
        }
    }

    /// Measure the assembled reply, including context and configuration suggestions. The normal
    /// previews have individual budgets, and a large live working set gets first claim on the frame.
    pub(crate) fn into_json(self, script_read: bool) -> Result<String> {
        self.encode(script_read, None)
    }

    pub(crate) fn into_paged_json(self, offset: u64, digest: &str) -> Result<String> {
        self.encode(false, Some((offset, digest)))
    }

    fn encode(mut self, script_read: bool, continuation: Option<(u64, &str)>) -> Result<String> {
        let encoded = self.value.to_string();
        if encoded.len() <= GET_JSON_BYTES { return Ok(encoded); }

        let reply = &mut self.value;
        for field in ["process_logs", "build_cleanup", "publication_history", "notice_deliveries", "notices"] {
            if let Some(rows) = reply[field].as_array_mut() {
                let count = rows.len();
                rows.clear();
                omitted(reply, &format!("{field}_omitted"), count);
            }
        }
        let encoded = reply.to_string();
        if encoded.len() <= GET_JSON_BYTES { return Ok(encoded); }
        if let Some((offset, expected_digest)) = continuation {
            let snapshot_digest = digest(&encoded);
            let offset = if snapshot_digest == expected_digest { offset } else { 0 };
            let page = chunk(&encoded, offset, GET_JSON_BYTES / 3, snapshot_digest)?;
            return Ok(json!({"schema": 1, "workflow_page": page}).to_string());
        }
        if reply["state"].is_null() { return Err(Error::Refused("workflow reply exceeds frame")); }

        let mut groups = 0;
        for id in self.preview.groups {
            for field in ["back_edge_budgets", "stops"] {
                if let Some(rows) = reply[field].as_object_mut() { rows.remove(&id); }
            }
            if let Some(items) = reply["state"]["items"].as_object_mut() && items.remove(&id).is_some() {
                groups += 1;
            }
        }
        let mut archive = 0;
        if let Some(rows) = reply["state"].get_mut("archive").and_then(Value::as_object_mut) {
            rows.retain(|id, row| {
                let keep = self.preview.dependencies.contains(id) && row["phase"] == "Done";
                // Keep only the facts the old presentation needs to resolve the dependency.
                if keep {
                    let marker = json!({"phase":"Done", "candidate":null});
                    if *row != marker {
                        if row.get("group").is_some() { groups += 1; } else { archive += 1; }
                        *row = marker;
                    }
                } else if row.get("group").is_some() { groups += 1; } else { archive += 1; }
                keep
            });
        }
        omitted(reply, "landing_groups_omitted", groups);
        omitted(reply, "archive_omitted", archive);
        let mut runs = 0;
        if let Some(items) = reply["state"]["items"].as_object_mut() {
            for item in items.values_mut().filter_map(Value::as_object_mut) {
                if let Some(Value::Array(history)) = item.remove("history") { runs += history.len(); }
            }
        }
        omitted(reply, "runs_omitted", runs);
        if let Some(seen) = reply["state"].get_mut("seen") { *seen = json!([]); }

        let encoded = reply.to_string();
        if encoded.len() <= GET_JSON_BYTES { return Ok(encoded); }

        // Current-run reasons can dwarf the rows carrying them. Keep the decisions so
        // shipped clients still show the correct send-back count and review state.
        let mut compact = false;
        if let Some(items) = reply["state"]["items"].as_object_mut() {
            for item in items.values_mut() {
                if let Some(fields) = item.as_object_mut() {
                    compact |= fields.remove("brief").is_some();
                }
                for history in ["sent_back", "rework_history"] {
                    if let Some(records) = item.get_mut(history).and_then(Value::as_array_mut) {
                        for record in records {
                            const POINTER: &str = "See workflow history for the full reason.";
                            if record["reason"].as_str().is_some_and(|text| text.len() > POINTER.len()) {
                                record["reason"] = POINTER.into();
                                compact = true;
                            }
                        }
                    }
                }
            }
        }
        if compact { reply["item_details_omitted"] = true.into(); }
        let encoded = reply.to_string();
        if encoded.len() <= GET_JSON_BYTES { return Ok(encoded); }

        if let Some(fields) = reply.as_object_mut() {
            for field in ["places", "back_edge_budgets", "stops"] { fields.remove(field); }
        }
        reply["state"]["archive"] = json!({});
        // The archive is now empty, including bare Done rows retained for dependencies.
        // Some markers already counted as omitted when their details were trimmed.
        reply["archive_omitted"] = self.preview.archive_total.into();
        let encoded = reply.to_string();
        if encoded.len() <= GET_JSON_BYTES { return Ok(encoded); }

        if script_read {
            if let Some(items) = reply["state"]["items"].as_object_mut() {
                for item in items.values_mut().filter_map(Value::as_object_mut) {
                    // Agent dispatch uses the tracker source and follower relationship to
                    // distinguish a ticket it can start from one already held by the engine.
                    let tracker = item.get("definition").is_some_and(|definition|
                        definition["source"] == farcooler_workflow::TICKET_SOURCE);
                    item.retain(|key, _| matches!(key.as_str(), "run" | "pin" | "phase" | "follows"));
                    if tracker {
                        item.insert("definition".into(), json!({"source": farcooler_workflow::TICKET_SOURCE}));
                    }
                }
            }
            reply["items_compact"] = true.into();
        } else {
            // Shipped apps need their own mutation guard in a single frame.
            // Preserve the complete map whenever the reply fits above.
            let raw = &reply["state"]["source_sequences"];
            if raw.to_string().len() > SOURCE_SEQUENCE_BYTES && let Some(sequences) = raw.as_object() {
                let caller = reply["context"]["capabilities"]["decision_principal"].as_str();
                let mut kept = Map::new();
                if let Some(source) = caller.map(|principal| format!("wire:{principal}"))
                    && let Some(sequence) = sequences.get(&source)
                {
                    kept.insert(source, sequence.clone());
                }
                reply["source_sequences_omitted"] = (sequences.len() - kept.len()).into();
                reply["state"]["source_sequences"] = kept.into();
            }
            fit_legacy_items(reply)?;
        }
        let encoded = reply.to_string();
        if encoded.len() > GET_JSON_BYTES { return Err(Error::Refused("workflow reply exceeds frame")); }
        Ok(encoded)
    }
}

fn fit_legacy_items(reply: &mut Value) -> Result<()> {
    let Some(items) = reply["state"]["items"].as_object_mut() else { return Ok(()); };
    let mut rows: Vec<_> = std::mem::take(items).into_iter().collect();
    // The shipped Plan decoder validates the definition against its pin. Keep whole,
    // decodable rows, giving working runs and joined tickets priority over tracker cards.
    rows.sort_by_key(|(_, item)| item["definition"]["source"] == farcooler_workflow::TICKET_SOURCE);
    let total = rows.len();
    reply["items_omitted"] = total.into();
    let mut budget = GET_JSON_BYTES.saturating_sub(reply.to_string().len());
    let items = reply["state"]["items"].as_object_mut().expect("the item map was emptied above");
    for (id, item) in rows {
        if spend(&mut budget, &(&id, &item))? { items.insert(id, item); }
    }
    reply["items_omitted"] = (total - items.len()).into();
    Ok(())
}

pub(super) enum ListOrder { OldestFirst, NewestFirst }

/// Spend the byte budget on the newest records first, preserving the existing output order.
pub(super) fn recent<T: Serialize>(reply: &mut Map<String, Value>, name: &str, mut list: Vec<T>,
    order: ListOrder, mut budget: usize) -> Result<()>
{
    let oldest_first = matches!(order, ListOrder::OldestFirst);
    if oldest_first { list.reverse(); }
    let total = list.len();
    let mut rows = Vec::new();
    for row in list.into_iter().take(50) {
        let row = serde_json::to_value(row)?;
        if !spend(&mut budget, &row)? { break; }
        rows.push(row);
    }
    if oldest_first { rows.reverse(); }
    reply.insert(format!("{name}_omitted"), (total - rows.len()).into());
    reply.insert(name.into(), Value::Array(rows));
    Ok(())
}

pub(super) fn archived_value(id: &str, item: &Archived) -> Result<Value> {
    let mut value = serde_json::to_value(item)?;
    if item.can_start() {
        let (run, pin) = item.start_identity(id);
        value["run"] = run.into();
        value["pin"] = pin.into();
    }
    Ok(value)
}

fn page(value: &Value, offset: u64) -> Result<Value> {
    // The source has already been JSON-escaped. Escaping the chunk as a JSON string
    // can at most double its length, leaving room for the envelope and metadata.
    let encoded = serde_json::to_string(value)?;
    chunk(&encoded, offset, HISTORY_CHUNK_BYTES, digest(&encoded))
}

fn chunk(encoded: &str, offset: u64, limit: usize, digest: String) -> Result<Value> {
    let start = usize::try_from(offset).map_err(|_| Error::Refused("invalid history offset"))?;
    if !encoded.is_char_boundary(start) { return Err(Error::Refused("invalid history offset")); }
    let mut end = (start + limit).min(encoded.len());
    while !encoded.is_char_boundary(end) { end -= 1; }
    Ok(json!({"schema": 1, "offset": start, "next_offset": (end < encoded.len()).then_some(end),
        "digest": digest, "chunk": &encoded[start..end]}))
}

impl Exec {
    fn history(&self, workspace: &str, section: HistorySection, item: Option<&str>, owner: bool) -> Result<Value> {
        let state = self.state(workspace)?;
        let matches_item = |key: &str| item.is_none_or(|item| key.eq_ignore_ascii_case(item));
        Ok(match section {
            HistorySection::Items => {
                let items: BTreeMap<_, _> = state.iter().flat_map(|s| &s.items)
                    .filter(|(id, row)| row.group_of.is_none() && matches_item(id)).collect();
                serde_json::to_value(items)?
            }
            HistorySection::Groups => {
                let groups: BTreeMap<_, _> = state.iter().flat_map(|s| &s.items)
                    .filter(|(id, row)| row.group_of.is_some() && matches_item(id)).collect();
                serde_json::to_value(groups)?
            }
            HistorySection::Archive => {
                let archive: BTreeMap<_, _> = state.iter().flat_map(|s| &s.archive).filter(|(id, _)| matches_item(id))
                    .map(|(id, row)| Ok((id, archived_value(id, row)?))).collect::<Result<_>>()?;
                serde_json::to_value(archive)?
            }
            HistorySection::Runs => {
                let runs: BTreeMap<_, _> = state.iter().flat_map(|s| &s.items)
                    .filter(|(id, row)| matches_item(id) && !row.history.is_empty())
                    .map(|(id, row)| (id, &row.history)).collect();
                serde_json::to_value(runs)?
            }
            HistorySection::SourceSequences => match state {
                Some(state) => serde_json::to_value(state.source_sequences)?,
                None => json!({}),
            },
            HistorySection::Cleanup => serde_json::to_value(self.store.workflow_cleanups(workspace)?)?,
            HistorySection::Publications => {
                let mut value = json!({"publication_history": self.store.workflow_publication_history(workspace)?});
                if !owner { super::wire_commands::hide_environment(&mut value); }
                value["publication_history"].take()
            }
            HistorySection::Notices => serde_json::to_value(self.store.workflow_notice_deliveries(workspace)?)?,
            HistorySection::Logs => serde_json::to_value(self.retained_process_logs(workspace, state.as_ref(), item)?)?,
        })
    }

    fn archive_item(&self, workspace: &str, key: &str) -> Result<Value> {
        let state = self.state(workspace)?;
        let Some(state) = state else { return Ok(json!({"schema": 1, "item": null})); };
        let mut rows = state.archive.iter().filter(|(id, _)| id.eq_ignore_ascii_case(key));
        let item = if let Some((id, row)) = rows.next() {
            if rows.next().is_some() { return Err(Error::Refused("ambiguous archive item")); }
            let identity = row.can_start().then(|| row.start_identity(id));
            json!({"key": id, "phase": row.phase, "run": identity.as_ref().map(|(run, _)| run),
                "pin": identity.as_ref().map(|(_, pin)| pin)})
        } else { Value::Null };
        Ok(json!({"schema": 1, "item": item, "revision": state.revision, "epoch": state.epoch}))
    }
}

impl CommandHandle {
    pub(crate) async fn read_history(&self, admission: Admission, action: Action) -> Result<Value> {
        if self.is_closed() { return Err(Error::Refused("runtime inbox closed")); }
        admission.recheck().and_then(|()| admission.permits(&action)).map_err(Error::Domain)?;
        let reader = self.reader.clone();
        let workspace = admission.workspace();
        let owner = admission.is_owner();
        let read = action.clone();
        let value = tokio::task::spawn_blocking(move || match read {
            Action::History { section, offset, item } => page(&reader.history(&workspace, section, item.as_deref(), owner)?, offset),
            Action::ArchiveItem { item } => reader.archive_item(&workspace, &item),
            _ => unreachable!("history reads only"),
        }).await.map_err(|_| Error::Refused("workflow read worker failed"))??;
        admission.recheck().and_then(|()| admission.permits(&action)).map_err(Error::Domain)?;
        if self.is_closed() { return Err(Error::Refused("runtime inbox closed")); }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_working_set_gets_the_frame_before_previews() {
        let live = json!({"phase":"WaitingApproval", "run":"run", "pin":"pin", "brief":"x".repeat(990 * 1024)});
        let value = json!({
            "state": {"items":{"ticket":live}, "epoch":1, "revision":42, "source_sequences":{"wire:owner":4}},
            "process_logs":[{"tail":"x".repeat(140 * 1024)}], "process_logs_omitted":9
        });
        let encoded = WorkflowRead { value, preview: Preview::default() }.into_json(false).unwrap();
        assert!(encoded.len() < farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES);
        let read: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(read["process_logs_omitted"], 10);
        assert_eq!(read["process_logs"], json!([]));
        assert_eq!(read["state"]["items"]["ticket"], live);
    }

    #[test]
    fn a_large_working_set_gets_the_frame_before_failed_groups() {
        let detailed = json!({"phase":"Failed", "run":"group-run", "pin":"group-pin", "brief":"x".repeat(80 * 1024 - 128)});
        let value = json!({"state": {"items": {
            "ticket": {"phase":"WaitingApproval", "run":"run", "pin":"pin", "brief":"x".repeat(990 * 1024)},
            "group": detailed,
        }}});
        for script_read in [true, false] {
            let preview = Preview {
                groups: vec!["group".into()],
                ..Preview::default()
            };
            let encoded = WorkflowRead { value: value.clone(), preview }.into_json(script_read).unwrap();
            let read: Value = serde_json::from_str(&encoded).unwrap();
            assert_eq!(read["landing_groups_omitted"], 1);
            assert!(read["state"]["items"].get("group").is_none());
            assert_eq!(read["state"]["items"]["ticket"], value["state"]["items"]["ticket"]);
        }
    }

    #[test]
    fn history_offsets_are_utf8_boundaries_and_preserve_every_byte() {
        let value = json!(format!("{}界", "é".repeat(HISTORY_CHUNK_BYTES / 2)));
        let first = page(&value, 0).unwrap();
        let next = first["next_offset"].as_u64().unwrap();
        assert_eq!(next, HISTORY_CHUNK_BYTES as u64 - 1);
        assert!(page(&value, next + 1).is_err());
        let second = page(&value, next).unwrap();
        assert!(second["next_offset"].is_null());
        assert_eq!(first["digest"], second["digest"]);
        assert_eq!(format!("{}{}", first["chunk"].as_str().unwrap(), second["chunk"].as_str().unwrap()), value.to_string());
    }
}
