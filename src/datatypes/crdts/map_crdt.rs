use std::{
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};

use rpds::RedBlackTreeMapSync;

use super::{LocalOperationOutcome, RollbackAction, timed_node::TimedNode};
use crate::{
    DatatypeError,
    datatypes::common::ReturnType,
    errors::{datatypes::InternalReason, messages::datatype as msg},
    operations::body::OperationBody,
    types::{operation_context::OperationContext, timestamp::Timestamp},
    utils::json::decode_json_value,
};

#[derive(PartialEq, Eq)]
pub struct MapCrdt {
    pub(super) entries: RedBlackTreeMapSync<Arc<str>, TimedNode>,
    pub(super) live_count: usize,
    pub(super) tombstone_count: usize,
    pub(super) tombstone_bytes: u64,
    logical_bytes: u64,
}

impl Default for MapCrdt {
    fn default() -> Self {
        Self {
            entries: RedBlackTreeMapSync::new_sync(),
            live_count: 0,
            tombstone_count: 0,
            tombstone_bytes: 0,
            logical_bytes: 0,
        }
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct MapRollbackAction {
    key: Arc<str>,
    previous: Option<TimedNode>,
}

impl Debug for MapRollbackAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRollbackAction")
            .field("key_size", &self.key.len())
            .field("previous", &self.previous)
            .finish()
    }
}

impl MapCrdt {
    pub(crate) fn fork_for_batch(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            live_count: self.live_count,
            tombstone_count: self.tombstone_count,
            tombstone_bytes: self.tombstone_bytes,
            logical_bytes: self.logical_bytes,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.live_count
    }

    pub(super) fn logical_state_bytes(&self) -> u64 {
        self.logical_bytes
    }

    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|entry| entry.v.is_some())
    }

    pub(crate) fn to_json(&self) -> Result<serde_json::Value, DatatypeError> {
        let mut object = serde_json::Map::new();
        for (key, entry) in self.entries.iter() {
            if let Some(value) = &entry.v {
                object.insert(key.to_string(), decode_json_value(value)?);
            }
        }
        Ok(serde_json::Value::Object(object))
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    pub fn shared_value(&self, key: &str) -> Option<Arc<[u8]>> {
        self.entries.get(key).and_then(|entry| entry.v.clone())
    }

    pub(super) fn replace(
        &mut self,
        key: Arc<str>,
        next: Option<TimedNode>,
    ) -> Result<Option<TimedNode>, DatatypeError> {
        let previous = self.entries.get(key.as_ref()).cloned();
        self.update_entry_stats(&key, previous.as_ref(), next.as_ref())?;

        if let Some(entry) = next {
            self.entries.insert_mut(key, entry);
        } else {
            self.entries.remove_mut(key.as_ref());
        }
        Ok(previous)
    }

    fn update_entry_stats(
        &mut self,
        key: &str,
        previous: Option<&TimedNode>,
        next: Option<&TimedNode>,
    ) -> Result<(), DatatypeError> {
        let previous_bytes = Self::entry_logical_bytes(key, previous)?;
        let next_bytes = Self::entry_logical_bytes(key, next)?;
        let logical_bytes = self
            .logical_bytes
            .checked_sub(previous_bytes)
            .and_then(|remaining| remaining.checked_add(next_bytes))
            .ok_or_else(|| execution_error(msg::STATE_SIZE_OVERFLOW))?;

        // Tombstone bytes are a subset of the checked total before and after replacement.
        if let Some(entry) = previous {
            if entry.v.is_some() {
                self.live_count -= 1;
            } else {
                self.tombstone_count -= 1;
                self.tombstone_bytes -= previous_bytes;
            }
        }
        if let Some(entry) = next {
            if entry.v.is_some() {
                self.live_count += 1;
            } else {
                self.tombstone_count += 1;
                self.tombstone_bytes += next_bytes;
            }
        }
        self.logical_bytes = logical_bytes;
        Ok(())
    }

    fn entry_logical_bytes(key: &str, entry: Option<&TimedNode>) -> Result<u64, DatatypeError> {
        let Some(entry) = entry else {
            return Ok(0);
        };
        entry
            .logical_bytes()
            .and_then(|bytes| bytes.checked_add(key.len() as u64))
            .ok_or_else(|| execution_error(msg::STATE_SIZE_OVERFLOW))
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        let (key, _) = context.map_input()?;
        if matches!(context.body(), OperationBody::MapRemove(_)) && self.shared_value(key).is_none()
        {
            return Ok(LocalOperationOutcome::NoOp {
                return_value: ReturnType::Map(None),
            });
        }
        if let Some(entry) = self.entries.get(key) {
            entry.validate_local_timestamp(context.timestamp())?;
        }
        let key: Arc<str> = key.into();
        let previous = match context.body() {
            OperationBody::MapPut(body) => {
                self.put(key.clone(), body.value.as_ref(), context.timestamp())
            }
            OperationBody::MapRemove(_) => self.remove(key.clone(), context.timestamp()),
            _ => return Err(execution_error(msg::UNSUPPORTED_OPERATION)),
        }?;
        let previous_value = previous.as_ref().and_then(|entry| entry.v.clone());
        Ok(LocalOperationOutcome::new(
            ReturnType::Map(previous_value),
            RollbackAction::Map(MapRollbackAction { key, previous }),
        ))
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        let (key, value) = context.map_input()?;
        if let Some(entry) = self.entries.get(key) {
            if !context.should_apply_lww(&entry.ts, entry.v.as_deref(), value)? {
                return Ok(());
            }
        }
        match context.body() {
            OperationBody::MapPut(body) => {
                self.put(key.into(), body.value.as_ref(), context.timestamp())?;
            }
            OperationBody::MapRemove(_) => {
                self.remove(key.into(), context.timestamp())?;
            }
            _ => return Err(execution_error(msg::UNSUPPORTED_OPERATION)),
        }
        Ok(())
    }

    fn put(
        &mut self,
        key: Arc<str>,
        value: &[u8],
        timestamp: &Timestamp,
    ) -> Result<Option<TimedNode>, DatatypeError> {
        self.replace(
            key,
            Some(TimedNode {
                ts: timestamp.clone(),
                v: Some(Arc::from(value)),
                approved_sseq_upper: None,
            }),
        )
    }

    fn remove(
        &mut self,
        key: Arc<str>,
        timestamp: &Timestamp,
    ) -> Result<Option<TimedNode>, DatatypeError> {
        self.replace(
            key,
            Some(TimedNode {
                ts: timestamp.clone(),
                v: None,
                approved_sseq_upper: None,
            }),
        )
    }

    pub(crate) fn apply_rollback_action(
        &mut self,
        action: MapRollbackAction,
    ) -> Result<(), DatatypeError> {
        self.replace(action.key, action.previous)?;
        Ok(())
    }
}

fn execution_error(message: &str) -> DatatypeError {
    InternalReason::ExecuteOperation(message.into()).into_error()
}

impl Debug for MapCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapCrdt")
            .field("live_count", &self.live_count)
            .field("tombstone_count", &self.tombstone_count)
            .field("tombstone_bytes", &self.tombstone_bytes)
            .finish()
    }
}

impl Display for MapCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(live:{}, tombstones:{}, tombstone_bytes:{})",
            self.live_count, self.tombstone_count, self.tombstone_bytes
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_reject_state_size_overflow_without_changing_the_map() {
        let mut map = MapCrdt {
            logical_bytes: u64::MAX,
            ..Default::default()
        };
        let before = map.fork_for_batch();
        let error = map
            .put("key".into(), b"null", &Timestamp::initial())
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "[E202] internal error: execute operation: logical state size exceeds its accounting range"
        );
        assert_eq!(map, before);
    }
}
