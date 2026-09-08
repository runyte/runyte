// SPDX-License-Identifier: MPL-2.0
use super::{Error, Header, MAX_MODEL_BYTES, MAX_ROWS, Model, Row, cancelled, invalid, limited};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Header>,
    #[serde(deserialize_with = "super::decode_operations")]
    pub operations: Vec<Operation>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    Insert { before: Option<String>, row: Row },
    Update { row: Row },
    Remove { ids: Vec<String> },
    Reorder { ids: Vec<String> },
}
// Internally tagged derive buffers a serde Content tree before field visitors.
// Decode the map directly so ID bounds apply while reading the original stream.
impl<'de> Deserialize<'de> for Operation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Insert,
            Update,
            Remove,
            Reorder,
        }
        #[derive(Deserialize)]
        struct Ids(#[serde(deserialize_with = "super::decoding::ids")] Vec<String>);
        struct OperationVisitor;
        impl<'de> Visitor<'de> for OperationVisitor {
            type Value = Operation;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a bounded view operation")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Operation, A::Error> {
                let mut kind = None;
                let mut before = None;
                let mut row = None;
                let mut ids = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "kind" => {
                            if kind.is_some() {
                                return Err(de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value::<Kind>()?);
                        }
                        "before" => {
                            if before.is_some() {
                                return Err(de::Error::duplicate_field("before"));
                            }
                            before = Some(map.next_value::<Option<String>>()?);
                        }
                        "row" => {
                            if row.is_some() {
                                return Err(de::Error::duplicate_field("row"));
                            }
                            row = Some(map.next_value::<Row>()?);
                        }
                        "ids" => {
                            if ids.is_some() {
                                return Err(de::Error::duplicate_field("ids"));
                            }
                            ids = Some(map.next_value::<Ids>()?.0);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["kind", "before", "row", "ids"],
                            ));
                        }
                    }
                }
                match kind.ok_or_else(|| de::Error::missing_field("kind"))? {
                    Kind::Insert if ids.is_none() => Ok(Operation::Insert {
                        before: before.ok_or_else(|| de::Error::missing_field("before"))?,
                        row: row.ok_or_else(|| de::Error::missing_field("row"))?,
                    }),
                    Kind::Update if ids.is_none() && before.is_none() => Ok(Operation::Update {
                        row: row.ok_or_else(|| de::Error::missing_field("row"))?,
                    }),
                    Kind::Remove if row.is_none() && before.is_none() => Ok(Operation::Remove {
                        ids: ids.ok_or_else(|| de::Error::missing_field("ids"))?,
                    }),
                    Kind::Reorder if row.is_none() && before.is_none() => Ok(Operation::Reorder {
                        ids: ids.ok_or_else(|| de::Error::missing_field("ids"))?,
                    }),
                    _ => Err(de::Error::custom("Fields do not match view operation kind")),
                }
            }
        }
        deserializer.deserialize_map(OperationVisitor)
    }
}

impl Operation {
    pub(super) fn references(&self) -> usize {
        match self {
            Self::Insert { before, .. } => 1 + usize::from(before.is_some()),
            Self::Update { .. } => 1,
            Self::Remove { ids } | Self::Reorder { ids } => ids.len(),
        }
    }
}
impl Model {
    pub fn patched(&self, patch: Patch, cancel: &AtomicBool) -> Result<Self, Error> {
        cancelled(cancel)?;
        if patch.operations.len() > 1024
            || patch
                .operations
                .iter()
                .map(Operation::references)
                .sum::<usize>()
                > super::MAX_PATCH_REFERENCES
        {
            return Err(limited("View patch operation limit exceeded"));
        }
        // Avoid repeated full-row copies while applying an ordered atomic patch.
        let mut order: Vec<_> = self.rows.iter().map(|row| row.id.clone()).collect();
        let mut rows: BTreeMap<_, _> = self
            .rows
            .iter()
            .map(|row| (row.id.clone(), row.clone()))
            .collect();
        let mut bytes: usize = rows.values().map(row_bytes).sum();
        for operation in patch.operations {
            cancelled(cancel)?;
            match operation {
                Operation::Insert { before, row } => {
                    if rows.contains_key(&row.id) {
                        return Err(invalid("Inserted view row already exists"));
                    }
                    if rows.len() == MAX_ROWS {
                        return Err(limited("View row limit exceeded"));
                    }
                    bytes = bytes.saturating_add(row_bytes(&row));
                    if bytes > MAX_MODEL_BYTES {
                        return Err(limited("View patch payload exceeds limit"));
                    }
                    let index = match before {
                        Some(before) => order
                            .iter()
                            .position(|id| id == &before)
                            .ok_or_else(|| invalid("View insertion anchor is missing"))?,
                        None => order.len(),
                    };
                    order.insert(index, row.id.clone());
                    rows.insert(row.id.clone(), row);
                }
                Operation::Update { row } => {
                    let old = rows
                        .get(&row.id)
                        .ok_or_else(|| invalid("Updated view row is missing"))?;
                    bytes = bytes - row_bytes(old) + row_bytes(&row);
                    if bytes > MAX_MODEL_BYTES {
                        return Err(limited("View patch payload exceeds limit"));
                    }
                    rows.insert(row.id.clone(), row);
                }
                Operation::Remove { ids } => {
                    if ids.len() > MAX_ROWS {
                        return Err(limited("View removal exceeds row limit"));
                    }
                    let removed: BTreeSet<_> = ids.iter().collect();
                    if removed.len() != ids.len() || ids.iter().any(|id| !rows.contains_key(id)) {
                        return Err(invalid("Removed view row is missing or duplicated"));
                    }
                    for id in &ids {
                        bytes -= row_bytes(&rows.remove(id).unwrap());
                    }
                    order.retain(|id| !removed.contains(id));
                }
                Operation::Reorder { ids } => {
                    if ids.len() != rows.len()
                        || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
                        || ids.iter().any(|id| !rows.contains_key(id))
                    {
                        return Err(invalid("View reorder must name every row exactly once"));
                    }
                    order = ids;
                }
            }
        }
        let mut model = Model {
            rows: order
                .into_iter()
                .map(|id| rows.remove(&id).unwrap())
                .collect(),
            ..self.clone_header()
        };
        if let Some(header) = patch.header {
            model.title = header.title;
            model.purpose = header.purpose;
            model.columns = header.columns;
            model.detail = header.detail;
            model.preview = header.preview;
            model.status = header.status;
            model.actions = header.actions;
        }
        cancelled(cancel)?;
        model.validate()?;
        Ok(model)
    }
    fn clone_header(&self) -> Self {
        Self {
            title: self.title.clone(),
            purpose: self.purpose,
            rows: vec![],
            columns: self.columns.clone(),
            detail: self.detail.clone(),
            preview: self.preview.clone(),
            status: self.status.clone(),
            actions: self.actions.clone(),
        }
    }
}
fn row_bytes(row: &Row) -> usize {
    row.id.len() + row.text.len() + row.cells.iter().map(|cell| cell.text.len()).sum::<usize>() + 1
}
