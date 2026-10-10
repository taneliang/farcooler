//! One transcript line, decoded leniently and only as far as the fold needs.
//!
//! Typed, borrowed and partial on purpose. `serde_json::Value` would allocate
//! every string in the record, and claude writes lines over a megabyte whose
//! bulk is a base64 image or a file dump nobody draws. Here a field the fold
//! does not name is skipped by the parser without being copied: an image
//! block's `source.data`, a thinking block's text, a tool result's `content`,
//! an attachment's whole body.
//!
//! Lenient in the shape of each field, too. A field that holds the wrong JSON
//! type (a `content` that is a number, a `toolUseResult` that is a string,
//! which claude writes for a refused tool) reads as absent rather than failing
//! the record. Only a line that is not JSON at all fails, and the fold turns
//! that into a `Gap`.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

/// A string, or nothing when the field held anything else.
#[derive(Debug, Default, Clone)]
pub(super) struct Str<'a>(pub Option<Cow<'a, str>>);

impl<'a> Str<'a> {
    pub fn get(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

/// A number, or nothing.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Num(pub Option<f64>);

impl Num {
    pub fn int(self) -> Option<i64> {
        self.0.filter(|n| n.is_finite()).map(|n| n as i64)
    }
}

/// A boolean, or nothing.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Bool(pub Option<bool>);

impl Bool {
    pub fn yes(self) -> bool {
        self.0 == Some(true)
    }
}

/// A JSON object decoded as `T`, or nothing when the field was not an object.
#[derive(Debug, Clone)]
pub(super) struct Obj<T>(pub Option<T>);

impl<T> Default for Obj<T> {
    fn default() -> Self {
        Obj(None)
    }
}

/// A JSON array of `T`, or empty when the field was not an array.
#[derive(Debug, Clone)]
pub(super) struct List<T>(pub Vec<T>);

impl<T> Default for List<T> {
    fn default() -> Self {
        List(Vec::new())
    }
}

/// A message's `content`: claude writes a bare string for a typed prompt and an
/// array of blocks for everything else.
#[derive(Debug, Default)]
pub(super) enum Content<'a> {
    #[default]
    None,
    Text(Cow<'a, str>),
    Blocks(Vec<Obj<Block<'a>>>),
}

/// Drains whatever value is next, so a visitor that rejects a type still
/// leaves the parser where the field ends.
fn drain_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
    while seq.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

fn drain_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

/// Every visitor below answers "nothing" to the JSON types it does not want.
/// One macro rather than five copies of the same eight methods.
macro_rules! nothing_for {
    ($value:ty; $($method:ident($arg:ty)),*) => {
        $(fn $method<E: de::Error>(self, _: $arg) -> Result<$value, E> { Ok(Default::default()) })*
    };
}

impl<'de: 'a, 'a> Deserialize<'de> for Str<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Str<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Str<'a>, E> {
                Ok(Str(Some(Cow::Borrowed(v))))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Str<'a>, E> {
                Ok(Str(Some(Cow::Owned(v.to_string()))))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Str<'a>, E> {
                Ok(Str(Some(Cow::Owned(v))))
            }
            nothing_for!(Str<'a>; visit_bool(bool), visit_i64(i64), visit_u64(u64), visit_f64(f64));
            fn visit_unit<E: de::Error>(self) -> Result<Str<'a>, E> {
                Ok(Str(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Str<'a>, A::Error> {
                drain_seq(seq).map(|()| Str(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Str<'a>, A::Error> {
                drain_map(map).map(|()| Str(None))
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Num;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Num, E> {
                Ok(Num(Some(v as f64)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Num, E> {
                Ok(Num(Some(v as f64)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Num, E> {
                Ok(Num(Some(v)))
            }
            nothing_for!(Num; visit_bool(bool), visit_str(&str));
            fn visit_unit<E: de::Error>(self) -> Result<Num, E> {
                Ok(Num(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Num, A::Error> {
                drain_seq(seq).map(|()| Num(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Num, A::Error> {
                drain_map(map).map(|()| Num(None))
            }
        }
        d.deserialize_any(V)
    }
}

impl<'de> Deserialize<'de> for Bool {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Bool;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Bool, E> {
                Ok(Bool(Some(v)))
            }
            nothing_for!(Bool; visit_i64(i64), visit_u64(u64), visit_f64(f64), visit_str(&str));
            fn visit_unit<E: de::Error>(self) -> Result<Bool, E> {
                Ok(Bool(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Bool, A::Error> {
                drain_seq(seq).map(|()| Bool(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Bool, A::Error> {
                drain_map(map).map(|()| Bool(None))
            }
        }
        d.deserialize_any(V)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Obj<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Obj<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Obj<T>, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(|t| Obj(Some(t)))
            }
            nothing_for!(Obj<T>; visit_bool(bool), visit_i64(i64), visit_u64(u64), visit_f64(f64), visit_str(&str));
            fn visit_unit<E: de::Error>(self) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Obj<T>, A::Error> {
                drain_seq(seq).map(|()| Obj(None))
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for List<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = List<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<List<T>, A::Error> {
                Vec::<T>::deserialize(SeqAccessDeserializer::new(seq)).map(List)
            }
            nothing_for!(List<T>; visit_bool(bool), visit_i64(i64), visit_u64(u64), visit_f64(f64), visit_str(&str));
            fn visit_unit<E: de::Error>(self) -> Result<List<T>, E> {
                Ok(List(Vec::new()))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<List<T>, A::Error> {
                drain_map(map).map(|()| List(Vec::new()))
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Content<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Content<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Content<'a>, E> {
                Ok(Content::Text(Cow::Borrowed(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Content<'a>, E> {
                Ok(Content::Text(Cow::Owned(v.to_string())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Content<'a>, E> {
                Ok(Content::Text(Cow::Owned(v)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Content<'a>, A::Error> {
                Vec::<Obj<Block<'a>>>::deserialize(SeqAccessDeserializer::new(seq)).map(Content::Blocks)
            }
            nothing_for!(Content<'a>; visit_bool(bool), visit_i64(i64), visit_u64(u64), visit_f64(f64));
            fn visit_unit<E: de::Error>(self) -> Result<Content<'a>, E> {
                Ok(Content::None)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Content<'a>, A::Error> {
                drain_map(map).map(|()| Content::None)
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// One line of a claude transcript, main or sidechain.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct Record<'a> {
    #[serde(rename = "type", borrow)]
    pub kind: Str<'a>,
    #[serde(borrow)]
    pub subtype: Str<'a>,
    #[serde(borrow)]
    pub uuid: Str<'a>,
    #[serde(borrow)]
    pub timestamp: Str<'a>,
    #[serde(rename = "promptId", borrow)]
    pub prompt_id: Str<'a>,
    #[serde(rename = "promptSource", borrow)]
    pub prompt_source: Str<'a>,
    /// Who started the turn, as claude 2.1.28x names it: `human`,
    /// `scheduled`, `peer`, `task_notification`, `auto_continuation`.
    #[serde(rename = "turnOrigin", borrow)]
    pub turn_origin: Str<'a>,
    /// The scheduled task (`CronCreate`) whose firing this prompt is.
    #[serde(rename = "scheduledTaskId", borrow)]
    pub scheduled_task_id: Str<'a>,
    #[serde(rename = "isMeta")]
    pub is_meta: Bool,
    #[serde(rename = "isCompactSummary")]
    pub is_compact_summary: Bool,
    #[serde(borrow)]
    pub message: Obj<Message<'a>>,
    #[serde(rename = "toolUseResult", borrow)]
    pub tool_use_result: Obj<ToolUseResult<'a>>,
    /// A `queue-operation`'s verb: `enqueue`, `dequeue`, `remove`.
    #[serde(borrow)]
    pub operation: Str<'a>,
    /// A `queue-operation`'s text, or a `system` record's.
    #[serde(borrow)]
    pub content: Str<'a>,
    #[serde(rename = "durationMs")]
    pub duration_ms: Num,
    #[serde(rename = "pendingBackgroundAgentCount")]
    pub pending_background: Num,
    #[serde(borrow)]
    pub error: Obj<ApiError<'a>>,
    #[serde(rename = "compactMetadata", borrow)]
    pub compact: Obj<CompactMetadata<'a>>,
    /// An `assistant` record that is claude reporting a failed request, not
    /// the model speaking (`model: "<synthetic>"`, `stop_sequence`).
    #[serde(rename = "isApiErrorMessage")]
    pub is_api_error: Bool,
    #[serde(borrow)]
    pub attachment: Obj<Attachment<'a>>,
    /// A `queue-operation remove`'s reason: `absorbed_mid_turn`,
    /// `delivered_to_agent`.
    #[serde(borrow)]
    pub reason: Str<'a>,
}

/// The one attachment the fold reads: `queued_command`, a queued message
/// claude took into the turn it was running.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct Attachment<'a> {
    #[serde(rename = "type", borrow)]
    pub kind: Str<'a>,
    #[serde(borrow)]
    pub prompt: Str<'a>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct Message<'a> {
    #[serde(borrow)]
    pub id: Str<'a>,
    #[serde(borrow)]
    pub stop_reason: Str<'a>,
    #[serde(borrow)]
    pub content: Content<'a>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct Block<'a> {
    #[serde(rename = "type", borrow)]
    pub kind: Str<'a>,
    #[serde(borrow)]
    pub text: Str<'a>,
    #[serde(borrow)]
    pub id: Str<'a>,
    #[serde(borrow)]
    pub name: Str<'a>,
    #[serde(borrow)]
    pub input: ToolInput<'a>,
    #[serde(borrow)]
    pub tool_use_id: Str<'a>,
    pub is_error: Bool,
    /// A `tool_result`'s text, already cut to what a row keeps.
    pub content: ResultText,
}

/// A `tool_use`'s input: the fields a summary is made of, and the object as
/// written, which a row opens to (ov-452). Read once as raw JSON, so the
/// fields borrow from it and nothing is copied that no row keeps.
#[derive(Debug, Default)]
pub(super) struct ToolInput<'a> {
    pub fields: Option<Input<'a>>,
    pub raw: Option<Cow<'a, str>>,
}

impl<'a> ToolInput<'a> {
    /// The input a hook's payload carried.
    pub fn owned(fields: Input<'a>, raw: Option<String>) -> ToolInput<'a> {
        ToolInput { fields: Some(fields), raw: raw.map(Cow::Owned) }
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for ToolInput<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <&'de serde_json::value::RawValue>::deserialize(d)?;
        let text = raw.get();
        if !text.starts_with('{') {
            return Ok(ToolInput::default());
        }
        let fields = serde_json::from_str::<Input<'de>>(text).ok();
        Ok(ToolInput { fields, raw: Some(Cow::Borrowed(text)) })
    }
}

/// A `tool_result`'s `content`: a string, or blocks whose text parts are
/// joined. Cut as it is read (`detail::excerpt`), so a file dump costs the
/// parser's pass over it and no copy.
#[derive(Debug, Default)]
pub(super) struct ResultText(pub Option<String>);

impl<'de> Deserialize<'de> for ResultText {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ResultText;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ResultText, E> {
                Ok(ResultText(super::detail::excerpt(v)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ResultText, A::Error> {
                let mut joined = String::new();
                while let Some(part) = seq.next_element::<Obj<ResultPart>>()? {
                    let Some(text) = part.0.and_then(|p| p.text.0) else { continue };
                    if joined.len() > super::detail::DETAIL_CHARS * 4 {
                        continue;
                    }
                    if !joined.is_empty() {
                        joined.push('\n');
                    }
                    joined.push_str(&text);
                }
                Ok(ResultText(super::detail::excerpt(&joined)))
            }
            nothing_for!(ResultText; visit_bool(bool), visit_i64(i64), visit_u64(u64), visit_f64(f64));
            fn visit_unit<E: de::Error>(self) -> Result<ResultText, E> {
                Ok(ResultText(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<ResultText, A::Error> {
                drain_map(map).map(|()| ResultText(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// One block of a `tool_result`'s content: its text, cut; an image's data is
/// skipped unread.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ResultPart {
    text: ResultText,
}

/// The few `tool_use.input` fields a row's summary line is made of.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct Input<'a> {
    #[serde(borrow)]
    pub description: Str<'a>,
    #[serde(borrow)]
    pub command: Str<'a>,
    #[serde(borrow)]
    pub file_path: Str<'a>,
    #[serde(borrow)]
    pub pattern: Str<'a>,
    #[serde(borrow)]
    pub path: Str<'a>,
    #[serde(borrow)]
    pub subject: Str<'a>,
    #[serde(borrow)]
    pub url: Str<'a>,
    #[serde(borrow)]
    pub query: Str<'a>,
    #[serde(borrow)]
    pub subagent_type: Str<'a>,
    pub run_in_background: Bool,
    #[serde(borrow)]
    pub questions: List<Obj<Question<'a>>>,
    #[serde(borrow)]
    pub plan: Str<'a>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct Question<'a> {
    #[serde(borrow)]
    pub question: Str<'a>,
    #[serde(borrow)]
    pub header: Str<'a>,
    #[serde(borrow)]
    pub options: List<Obj<QuestionOption<'a>>>,
    #[serde(rename = "multiSelect")]
    pub multi_select: Bool,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct QuestionOption<'a> {
    #[serde(borrow)]
    pub label: Str<'a>,
    #[serde(borrow)]
    pub description: Str<'a>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct ToolUseResult<'a> {
    #[serde(rename = "agentId", borrow)]
    pub agent_id: Str<'a>,
    #[serde(rename = "agentType", borrow)]
    pub agent_type: Str<'a>,
    #[serde(borrow)]
    pub status: Str<'a>,
    #[serde(rename = "structuredPatch", borrow)]
    pub structured_patch: List<Obj<Hunk<'a>>>,
    #[serde(rename = "filePath", borrow)]
    pub file_path: Str<'a>,
    #[serde(rename = "totalToolUseCount")]
    pub total_tool_use_count: Num,
    #[serde(rename = "totalDurationMs")]
    pub total_duration_ms: Num,
    pub interrupted: Bool,
    /// A `TaskCreate`'s new task: its `id` is what `TaskUpdate` names.
    #[serde(borrow)]
    pub task: Obj<TaskRef<'a>>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct TaskRef<'a> {
    #[serde(borrow)]
    pub id: Str<'a>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct Hunk<'a> {
    #[serde(rename = "oldStart")]
    pub old_start: Num,
    #[serde(rename = "oldLines")]
    pub old_lines: Num,
    #[serde(rename = "newStart")]
    pub new_start: Num,
    #[serde(rename = "newLines")]
    pub new_lines: Num,
    #[serde(borrow)]
    pub lines: List<Str<'a>>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct ApiError<'a> {
    #[serde(borrow)]
    pub formatted: Str<'a>,
    #[serde(borrow)]
    pub message: Str<'a>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct CompactMetadata<'a> {
    #[serde(borrow)]
    pub trigger: Str<'a>,
}

/// A subagent's `agent-<id>.meta.json`, the join onto the parent's `Agent` call.
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(default)]
pub struct SubagentMeta {
    #[serde(rename = "toolUseId")]
    pub tool_use_id: Option<String>,
    #[serde(rename = "agentType")]
    pub agent_type: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "requestShape")]
    pub request_shape: Option<String>,
}

/// Decode one line, or `None` when it is not a JSON object at all.
pub(super) fn decode(line: &[u8]) -> Option<Record<'_>> {
    serde_json::from_slice::<Obj<Record<'_>>>(line).ok()?.0
}
