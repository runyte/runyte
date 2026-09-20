// SPDX-License-Identifier: MPL-2.0
//! Reject oversized structural arrays while streaming, before retaining their allocations.
use super::{Cell, Column, MAX_COLUMNS, MAX_PATCH_REFERENCES, MAX_ROWS, Operation, Row};
use serde::{
    Deserialize, Deserializer,
    de::{self, IgnoredAny, SeqAccess, Visitor},
};
use std::{fmt, marker::PhantomData};

fn sequence<'de, D, T>(
    deserializer: D,
    limit: usize,
    budget: usize,
    weight: fn(&T) -> usize,
) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T> {
        limit: usize,
        budget: usize,
        weight: fn(&T) -> usize,
        marker: PhantomData<T>,
    }
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Bounded<T> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a bounded view array")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            let mut retained = 0usize;
            loop {
                if values.len() == self.limit {
                    if sequence.next_element::<IgnoredAny>()?.is_some() {
                        return Err(de::Error::custom("View array limit exceeded"));
                    }
                    break;
                }
                let Some(value) = sequence.next_element::<T>()? else {
                    break;
                };
                retained = retained.saturating_add((self.weight)(&value));
                if retained > self.budget {
                    return Err(de::Error::custom("View patch reference limit exceeded"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Bounded {
        limit,
        budget,
        weight,
        marker: PhantomData,
    })
}

macro_rules! bounded {
    ($name:ident, $element:ty, $limit:expr) => {
        pub(crate) fn $name<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<$element>, D::Error> {
            sequence(deserializer, $limit, $limit, |_| 1)
        }
    };
}
bounded!(rows, Row, MAX_ROWS);
bounded!(cells, Cell, MAX_COLUMNS);
bounded!(columns, Column, MAX_COLUMNS);
bounded!(actions, String, super::super::application::MAX_COMMANDS);
pub(crate) fn row_actions<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    actions(deserializer).map(Some)
}
bounded!(ids, String, MAX_ROWS);

pub(crate) fn operations<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Operation>, D::Error> {
    sequence(
        deserializer,
        1024,
        MAX_PATCH_REFERENCES,
        Operation::references,
    )
}

pub(crate) fn metadata<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<super::Metadata>>, D::Error> {
    sequence(deserializer, 16, 16, |_| 1).map(Some)
}

pub(crate) fn presentations<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<
    Option<std::collections::BTreeMap<String, super::super::presentation::Presentation>>,
    D::Error,
> {
    use serde::de::MapAccess;
    use std::collections::BTreeMap;
    struct Presentations;
    impl<'de> Visitor<'de> for Presentations {
        type Value = BTreeMap<String, super::super::presentation::Presentation>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a bounded map of view command presentations")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut entries = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if entries.len() == super::super::application::MAX_COMMANDS {
                    return Err(de::Error::custom("View action presentation limit exceeded"));
                }
                if entries.contains_key(&key) {
                    return Err(de::Error::custom(
                        "Duplicate view action presentation command",
                    ));
                }
                entries.insert(key, map.next_value()?);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_map(Presentations).map(Some)
}

pub(crate) fn document<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
