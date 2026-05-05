use std::{fmt::Debug, io::Write};

use tracing::field::{Field, Visit};

const MESSAGE_FIELD: &str = "message";
const COLLECTION_FIELD: &str = "collection";
const CLIENT_FIELD: &str = "client";
const CUID_FIELD: &str = "cuid";
const DATATYPE_FIELD: &str = "data_key";
const DUID_FIELD: &str = "duid";

#[derive(Default, Debug)]
pub struct QortooTraceContextVisitor {
    msg: Vec<u8>,
    collection: Vec<u8>,
    client: Vec<u8>,
    cuid: Vec<u8>,
    datatype: Vec<u8>,
    duid: Vec<u8>,
}

impl QortooTraceContextVisitor {
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    #[inline]
    pub fn message_into(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(self.msg.as_ref());
        write!(buf, "\t\t").unwrap();
    }

    #[inline]
    pub fn category_into(&self, buf: &mut Vec<u8>) {
        if !self.collection.is_empty() {
            write!(buf, "🗄").unwrap();
            buf.extend_from_slice(self.collection.as_ref());
        }
        if !self.client.is_empty() || !self.cuid.is_empty() {
            write!(buf, "👥").unwrap();
            buf.extend_from_slice(self.client.as_ref());
            write!(buf, "(").unwrap();
            buf.extend_from_slice(self.cuid.as_ref());
            write!(buf, ")").unwrap();
        }
        if !self.datatype.is_empty() || !self.duid.is_empty() {
            write!(buf, "🗂").unwrap();
            buf.extend_from_slice(self.datatype.as_ref());
            write!(buf, "(").unwrap();
            buf.extend_from_slice(self.duid.as_ref());
            write!(buf, ")").unwrap();
        }
        write!(buf, "\t").unwrap();
    }

    /// Fills any empty context fields from `other`, walking from the innermost span outward.
    ///
    /// Returns `true` to signal the caller to keep traversing ancestor spans (at least one
    /// field is still missing), or `false` once all five context fields are collected and
    /// further traversal would yield nothing new.
    pub fn merge(&mut self, other: &Self) -> bool {
        // All five context fields are present — no need to walk further up the span tree.
        if !self.collection.is_empty()
            && !self.client.is_empty()
            && !self.cuid.is_empty()
            && !self.datatype.is_empty()
            && !self.duid.is_empty()
        {
            return false;
        }

        if self.collection.is_empty() && !other.collection.is_empty() {
            self.collection = other.collection.clone();
        }
        if self.client.is_empty() && !other.client.is_empty() {
            self.client = other.client.clone();
        }
        if self.cuid.is_empty() && !other.cuid.is_empty() {
            self.cuid = other.cuid.clone();
        }
        if self.datatype.is_empty() && !other.datatype.is_empty() {
            self.datatype = other.datatype.clone();
        }
        if self.duid.is_empty() && !other.duid.is_empty() {
            self.duid = other.duid.clone();
        }
        true
    }
}

impl Visit for QortooTraceContextVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            MESSAGE_FIELD => self.msg.extend_from_slice(value.as_bytes()),
            COLLECTION_FIELD => self.collection.extend_from_slice(value.as_bytes()),
            CLIENT_FIELD => self.client.extend_from_slice(value.as_bytes()),
            CUID_FIELD => self.cuid.extend_from_slice(value.as_bytes()),
            DATATYPE_FIELD => self.datatype.extend_from_slice(value.as_bytes()),
            DUID_FIELD => self.duid.extend_from_slice(value.as_bytes()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let _ = match field.name() {
            MESSAGE_FIELD => write!(self.msg, "{:?}", value),
            COLLECTION_FIELD => write!(self.collection, "{:?}", value),
            CLIENT_FIELD => write!(self.client, "{:?}", value),
            CUID_FIELD => write!(self.cuid, "{:?}", value),
            DATATYPE_FIELD => write!(self.datatype, "{:?}", value),
            DUID_FIELD => write!(self.duid, "{:?}", value),
            _ => Ok(()),
        };
    }
}
