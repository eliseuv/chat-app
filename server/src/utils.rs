use std::{
    collections::{hash_map, HashMap},
    fmt::Display,
    hash::Hash,
};

const SAFE_MODE: bool = false;

pub struct Sensitive<T: Display>(pub T);
impl<T: Display> Display for Sensitive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if SAFE_MODE {
            write!(f, "[REDACTED]")
        } else {
            write!(f, "{value}", value = self.0)
        }
    }
}

// Get mutable reference to newly inserted value or to existing value in `HashMap`
pub fn insert_or_get_mut<K, V>(hashmap: &mut HashMap<K, V>, key: K, new_value: V) -> &mut V
where
    K: Eq + Hash,
{
    match hashmap.entry(key) {
        hash_map::Entry::Occupied(entry) => entry.into_mut(),
        hash_map::Entry::Vacant(entry) => entry.insert(new_value),
    }
}
