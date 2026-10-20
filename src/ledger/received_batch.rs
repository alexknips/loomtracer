//! What the graph takes from a received transcript batch, read without decoding the transcript.
//!
//! An `ingest.batch_received` event is the raw conversation a capture shipped, and on a cell it
//! is 97% of the ledger. The graph never holds one; a classification that names a batch reads
//! four small things from it (which batch it was, which session shipped it, with which agent tool,
//! and when its newest turn was said), so a replay reads those and leaves the turn text alone:
//! decoding every transcript whole, to read a few strings out of it and drop the rest, is most of
//! what a graph build spent on them.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::events::{
    received_batch_newest_turn_time, received_batch_session_id, Event, EventId, EventType,
};
use crate::Result;

use super::backend_error::storage_error;

/// One received batch as a replay needs it: who shipped it and the facts a classification reads.
/// Each field says what the payload held and nothing is guessed for what it did not: a batch with
/// no `batch_id` names nothing, a batch with no turn time has none, and so on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedBatchFacts {
    pub event_id: EventId,
    /// Who shipped the batch: the actor on the event.
    pub actor_id: String,
    /// The `batch_id` a classification names it by, when the payload holds a string there.
    pub batch_id: Option<String>,
    /// The capture session that shipped it, `None` when absent or blank
    /// ([`received_batch_session_id`]).
    pub session_id: Option<String>,
    /// The agent tool the payload names, empty when it names none.
    pub agent_tool: String,
    /// The newest of its turns' times ([`received_batch_newest_turn_time`]).
    pub newest_turn_time: Option<DateTime<Utc>>,
}

impl ReceivedBatchFacts {
    /// The facts of a decoded received batch. `None` for any other event, or one without an id.
    ///
    /// This is the reading the others are held to: [`Self::from_payload_text`] must agree with it
    /// on every payload.
    pub fn from_event(event: &Event) -> Option<Self> {
        if event.event_type != EventType::IngestBatchReceived {
            return None;
        }
        Some(Self {
            event_id: event.event_id?,
            actor_id: event.actor_id.clone(),
            batch_id: event
                .payload
                .get("batch_id")
                .and_then(|id| id.as_str())
                .map(str::to_owned),
            session_id: received_batch_session_id(&event.payload).map(str::to_owned),
            agent_tool: event
                .payload
                .get("agent_tool")
                .and_then(|tool| tool.as_str())
                .unwrap_or_default()
                .to_owned(),
            newest_turn_time: received_batch_newest_turn_time(&event.payload),
        })
    }

    /// The facts of the received batch whose payload is the JSON text `payload`, read in one pass
    /// that allocates for the few strings it keeps and skips the rest (the turn text) unparsed.
    /// What the SQL ledgers use, which hold the payload as text.
    pub fn from_payload_text(event_id: EventId, actor_id: String, payload: &str) -> Result<Self> {
        let lean: Lean = serde_json::from_str(payload).map_err(|error| {
            storage_error(format!(
                "event {event_id} has a payload that is not JSON: {error}"
            ))
        })?;
        Ok(Self {
            event_id,
            actor_id,
            batch_id: lean.batch_id,
            session_id: lean.session_id.filter(|id| !id.trim().is_empty()),
            agent_tool: lean.agent_tool.unwrap_or_default(),
            newest_turn_time: lean.newest_turn_time,
        })
    }
}

/// The parts of a payload the facts come from. Anything else, and anything of another shape than
/// the one asked for, is skipped, as `serde_json::Value::get(..).as_str()` skips it; a key that
/// repeats keeps its last value, as a decoded `Value` does.
#[derive(Default)]
struct Lean {
    batch_id: Option<String>,
    session_id: Option<String>,
    agent_tool: Option<String>,
    newest_turn_time: Option<DateTime<Utc>>,
}

impl<'de> Deserialize<'de> for Lean {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(LeanVisitor)
    }
}

struct LeanVisitor;

impl<'de> Visitor<'de> for LeanVisitor {
    type Value = Lean;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON payload")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Lean, A::Error> {
        let mut lean = Lean::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "batch_id" => lean.batch_id = map.next_value::<StringOrNothing>()?.0,
                "session_id" => lean.session_id = map.next_value::<StringOrNothing>()?.0,
                "agent_tool" => lean.agent_tool = map.next_value::<StringOrNothing>()?.0,
                "turns" => lean.newest_turn_time = map.next_value::<NewestTurnTime>()?.0,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(lean)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Lean, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Lean::default())
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<Lean, E> {
        Ok(Lean::default())
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<Lean, E> {
        Ok(Lean::default())
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<Lean, E> {
        Ok(Lean::default())
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Lean, E> {
        Ok(Lean::default())
    }

    fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<Lean, E> {
        Ok(Lean::default())
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<Lean, E> {
        Ok(Lean::default())
    }
}

/// The string a value is, or nothing for a value of any other kind.
struct StringOrNothing(Option<String>);

impl<'de> Deserialize<'de> for StringOrNothing {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(StringOrNothingVisitor)
    }
}

struct StringOrNothingVisitor;

impl<'de> Visitor<'de> for StringOrNothingVisitor {
    type Value = StringOrNothing;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<StringOrNothing, E> {
        Ok(StringOrNothing(Some(value.to_owned())))
    }

    fn visit_string<E: de::Error>(self, value: String) -> std::result::Result<StringOrNothing, E> {
        Ok(StringOrNothing(Some(value)))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<StringOrNothing, E> {
        Ok(StringOrNothing(None))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<StringOrNothing, E> {
        Ok(StringOrNothing(None))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<StringOrNothing, E> {
        Ok(StringOrNothing(None))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<StringOrNothing, E> {
        Ok(StringOrNothing(None))
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<StringOrNothing, E> {
        Ok(StringOrNothing(None))
    }

    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<StringOrNothing, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(StringOrNothing(None))
    }

    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<StringOrNothing, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(StringOrNothing(None))
    }
}

/// The newest time among the `ts` strings of a `turns` array that parse as RFC 3339, nothing for a
/// `turns` of any other kind.
struct NewestTurnTime(Option<DateTime<Utc>>);

impl<'de> Deserialize<'de> for NewestTurnTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(NewestTurnTimeVisitor)
    }
}

struct NewestTurnTimeVisitor;

impl<'de> Visitor<'de> for NewestTurnTimeVisitor {
    type Value = NewestTurnTime;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<NewestTurnTime, A::Error> {
        let mut newest: Option<DateTime<Utc>> = None;
        while let Some(TurnTime(time)) = seq.next_element::<TurnTime>()? {
            newest = newest.max(time);
        }
        Ok(NewestTurnTime(newest))
    }

    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<NewestTurnTime, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(NewestTurnTime(None))
    }

    fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<NewestTurnTime, E> {
        Ok(NewestTurnTime(None))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<NewestTurnTime, E> {
        Ok(NewestTurnTime(None))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<NewestTurnTime, E> {
        Ok(NewestTurnTime(None))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<NewestTurnTime, E> {
        Ok(NewestTurnTime(None))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<NewestTurnTime, E> {
        Ok(NewestTurnTime(None))
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<NewestTurnTime, E> {
        Ok(NewestTurnTime(None))
    }
}

/// The time of one turn: its `ts` when that is a string holding an RFC 3339 time.
struct TurnTime(Option<DateTime<Utc>>);

impl<'de> Deserialize<'de> for TurnTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(TurnTimeVisitor)
    }
}

struct TurnTimeVisitor;

impl<'de> Visitor<'de> for TurnTimeVisitor {
    type Value = TurnTime;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<TurnTime, A::Error> {
        let mut time = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "ts" {
                time = map
                    .next_value::<StringOrNothing>()?
                    .0
                    .and_then(|ts| DateTime::parse_from_rfc3339(&ts).ok())
                    .map(|ts| ts.with_timezone(&Utc));
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(TurnTime(time))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<TurnTime, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(TurnTime(None))
    }

    fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<TurnTime, E> {
        Ok(TurnTime(None))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<TurnTime, E> {
        Ok(TurnTime(None))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<TurnTime, E> {
        Ok(TurnTime(None))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<TurnTime, E> {
        Ok(TurnTime(None))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<TurnTime, E> {
        Ok(TurnTime(None))
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<TurnTime, E> {
        Ok(TurnTime(None))
    }
}

#[cfg(test)]
mod tests;
