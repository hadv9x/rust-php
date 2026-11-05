//! Ordered PHP array keys and storage semantics.

use std::ops::{Deref, DerefMut};

use crate::Value;

/// PHP array keys retain their integer/string distinction and insertion order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// An integer or string key in the ordered PHP array representation.
pub enum ArrayKey {
    Integer(i64),
    String(String),
}

/// Ordered PHP array storage, including the historical next-free integer key.
#[derive(Clone, Debug)]
/// An ordered PHP array with next-free-integer-key tracking.
pub struct PhpArray {
    /// Entries stay ordered to preserve foreach and array-function behavior.
    pub(crate) entries: Vec<(ArrayKey, Value)>,
    /// Deleting the highest key does not rewind PHP's append index.
    next_free_index: Option<i64>,
    /// Array-internal iteration pointer used by `current`, `next`, `prev`, and related builtins.
    pub(crate) internal_pointer: isize,
    /// True only on the temporary array produced while expanding a Traversable call argument.
    pub(crate) was_traversable_unpack: bool,
}

impl PhpArray {
    /// Create an empty ordered array with its append index set to zero.
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new(), next_free_index: Some(0), internal_pointer: 0, was_traversable_unpack: false }
    }

    /// Rebuild ordered storage and derive the next append index from integer keys.
    pub fn from_entries(entries: Vec<(ArrayKey, Value)>) -> Self {
        // Reconstruct append state from visible integer keys when an operation creates an array.
        let largest_index = entries.iter().filter_map(|(key, _)| match key {
            ArrayKey::Integer(index) if *index >= 0 => Some(*index),
            _ => None,
        }).max();
        let next_free_index = match largest_index {
            Some(index) => index.checked_add(1),
            None => Some(0),
        };
        Self { entries, next_free_index, internal_pointer: 0, was_traversable_unpack: false }
    }

    /// Insert or replace a key while preserving its original position.
    pub fn set(&mut self, key: ArrayKey, value: Value) {
        if let ArrayKey::Integer(index) = key {
            if self.next_free_index.is_some_and(|next| index >= next) {
                self.next_free_index = index.checked_add(1);
            }
            self.set_existing_or_insert(ArrayKey::Integer(index), value);
        } else {
            self.set_existing_or_insert(key, value);
        }
    }

    /// Replace an existing key in place or append a new key at the end.
    fn set_existing_or_insert(&mut self, key: ArrayKey, value: Value) {
        if let Some((_, existing)) = self.entries.iter_mut().find(|(candidate, _)| *candidate == key) {
            if let Value::Reference(reference) = existing {
                *reference.borrow_mut() = value.dereference();
            } else {
                *existing = value;
            }
        } else {
            self.entries.push((key, value));
        }
    }

    /// Replace a slot with an explicit reference cell without assigning through an older cell.
    pub(crate) fn set_reference(&mut self, key: ArrayKey, reference: std::rc::Rc<std::cell::RefCell<Value>>) {
        if let ArrayKey::Integer(index) = &key {
            if self.next_free_index.is_some_and(|next| *index >= next) {
                self.next_free_index = index.checked_add(1);
            }
        }
        let value = Value::Reference(reference);
        if let Some((_, existing)) = self.entries.iter_mut().find(|(candidate, _)| *candidate == key) {
            *existing = value;
        } else {
            self.entries.push((key, value));
        }
    }

    /// Append at PHP’s next-free integer index, reporting index exhaustion.
    pub fn append(&mut self, value: Value) -> Result<(), String> {
        let index = self.next_free_index.ok_or("Cannot add element: next free integer key is exhausted")?;
        self.entries.push((ArrayKey::Integer(index), value));
        self.next_free_index = index.checked_add(1);
        Ok(())
    }

    pub(crate) fn append_reference(&mut self, reference: std::rc::Rc<std::cell::RefCell<Value>>) -> Result<(), String> {
        let index = self.next_free_index.ok_or("Cannot add element: next free integer key is exhausted")?;
        self.entries.push((ArrayKey::Integer(index), Value::Reference(reference)));
        self.next_free_index = index.checked_add(1);
        Ok(())
    }

    /// Remove an existing key without rewinding the append index.
    pub(crate) fn unset(&mut self, key: &ArrayKey) {
        if let Some(index) = self.entries.iter().position(|(candidate, _)| candidate == key) {
            self.entries.remove(index);
            if (index as isize) < self.internal_pointer { self.internal_pointer -= 1; }
            else if self.internal_pointer >= self.entries.len() as isize && !self.entries.is_empty() {
                self.internal_pointer = self.entries.len() as isize - 1;
            }
        }
        // Keep the historical next-free index after deleting an element, as PHP does.
    }
}

impl Deref for PhpArray {
    type Target = [(ArrayKey, Value)];
    /// Expose ordered entries to read-only array algorithms.
    fn deref(&self) -> &Self::Target { &self.entries }
}

impl DerefMut for PhpArray {
    /// Expose ordered entries to controlled in-place array algorithms.
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.entries }
}

impl IntoIterator for PhpArray {
    type Item = (ArrayKey, Value);
    type IntoIter = std::vec::IntoIter<Self::Item>;
    /// Consume the array while retaining its insertion order.
    fn into_iter(self) -> Self::IntoIter { self.entries.into_iter() }
}
