use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Go slice on a JSON boundary, retaining the observable `nil` versus empty distinction.
#[derive(Clone, Debug)]
pub struct GoSlice<T> {
    nil: bool,
    values: Vec<T>,
}

impl<T: PartialEq> PartialEq for GoSlice<T> {
    fn eq(&self, other: &Self) -> bool {
        self.nil == other.nil && self.values == other.values
    }
}

impl<T: Eq> Eq for GoSlice<T> {}

impl<T> Default for GoSlice<T> {
    fn default() -> Self {
        Self {
            nil: true,
            values: Vec::new(),
        }
    }
}

impl<T> GoSlice<T> {
    #[must_use]
    pub const fn is_nil(&self) -> bool {
        self.nil
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn make_non_nil(&mut self) {
        self.nil = false;
    }
}

impl<T> Deref for GoSlice<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<T> DerefMut for GoSlice<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.nil = false;
        &mut self.values
    }
}

impl<T> From<Vec<T>> for GoSlice<T> {
    fn from(value: Vec<T>) -> Self {
        Self {
            nil: false,
            values: value,
        }
    }
}

impl<T> FromIterator<T> for GoSlice<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            nil: false,
            values: iter.into_iter().collect(),
        }
    }
}

impl<'a, T> IntoIterator for &'a GoSlice<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut GoSlice<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T: Serialize> Serialize for GoSlice<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.nil {
            serializer.serialize_none()
        } else {
            self.values.serialize(serializer)
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for GoSlice<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<Vec<T>>::deserialize(deserializer)? {
            Some(values) => Self { nil: false, values },
            None => Self::default(),
        })
    }
}

/// A Go map on a JSON boundary, retaining the observable `nil` versus empty distinction.
#[derive(Clone, Debug)]
pub struct GoMap<K, V> {
    nil: bool,
    values: BTreeMap<K, V>,
}

impl<K: Ord, V: PartialEq> PartialEq for GoMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.nil == other.nil && self.values == other.values
    }
}

impl<K: Ord, V: Eq> Eq for GoMap<K, V> {}

impl<K, V> Default for GoMap<K, V> {
    fn default() -> Self {
        Self {
            nil: true,
            values: BTreeMap::new(),
        }
    }
}

impl<K, V> GoMap<K, V> {
    #[must_use]
    pub const fn is_nil(&self) -> bool {
        self.nil
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn make_non_nil(&mut self) {
        self.nil = false;
    }
}

impl<K: Ord, V> Deref for GoMap<K, V> {
    type Target = BTreeMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<K: Ord, V> DerefMut for GoMap<K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.nil = false;
        &mut self.values
    }
}

impl<K, V> From<BTreeMap<K, V>> for GoMap<K, V> {
    fn from(value: BTreeMap<K, V>) -> Self {
        Self {
            nil: false,
            values: value,
        }
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for GoMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            nil: false,
            values: iter.into_iter().collect(),
        }
    }
}

impl<'a, K: Ord, V> IntoIterator for &'a GoMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K: Ord + Serialize, V: Serialize> Serialize for GoMap<K, V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.nil {
            serializer.serialize_none()
        } else {
            self.values.serialize(serializer)
        }
    }
}

impl<'de, K: Ord + Deserialize<'de>, V: Deserialize<'de>> Deserialize<'de> for GoMap<K, V> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<BTreeMap<K, V>>::deserialize(deserializer)? {
            Some(values) => Self { nil: false, values },
            None => Self::default(),
        })
    }
}
