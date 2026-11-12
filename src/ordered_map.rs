use std::collections::{BTreeSet, HashMap};

pub struct OrderedMap<K, O, V> {
    values: HashMap<K, (O, V)>,
    order: BTreeSet<(O, K)>,
}

impl<K, O, V> OrderedMap<K, O, V> {
    pub fn new() -> Self {
        OrderedMap {
            values: HashMap::new(),
            order: BTreeSet::new(),
        }
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

impl<K: Eq + std::hash::Hash + Ord + Clone, O: Ord + Clone, V> OrderedMap<K, O, V> {
    pub fn insert(&mut self, key: K, ord: O, val: V) {
        if let Some((old_ord, _)) = self.values.insert(key.clone(), (ord.clone(), val)) {
            self.order.remove(&(old_ord, key.clone()));
        }
        self.order.insert((ord, key));
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.values.get(key).map(|(_, v)| v)
    }

    pub fn last(&self) -> Option<(&K, &V)> {
        if let Some(key) = self.order.last().map(|(_, v)| v) {
            Some((key, self.values.get(key).map(|(_, v)| v).unwrap()))
        } else {
            None
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (ord, v) = self.values.remove(key)?;
        self.order.remove(&(ord, key.clone()));
        Some(v)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.values.contains_key(key)
    }

    // pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
    //     self.order.iter().map(|(_, k)| (k, &self.values[k].1))
    // }

    pub fn iter(&self) -> OrderedMapIter<'_, K, O, V> {
        OrderedMapIter {
            map: self,
            order: self.order.iter(),
        }
    }
}

pub struct OrderedMapIter<'a, K, O, V> {
    map: &'a OrderedMap<K, O, V>,
    order: std::collections::btree_set::Iter<'a, (O, K)>,
}

impl<'a, K, O, V> Iterator for OrderedMapIter<'a, K, O, V>
where
    K: Eq + std::hash::Hash + Ord,
    O: Ord,
{
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let (_, key) = self.order.next()?;
        Some((key, &self.map.values[key].1))
    }
}

impl<'a, K, O, V> DoubleEndedIterator for OrderedMapIter<'a, K, O, V>
where
    K: Eq + std::hash::Hash + Ord,
    O: Ord,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        let (_, key) = self.order.next_back()?;
        Some((key, &self.map.values[key].1))
    }
}

impl<'a, K, O, V> IntoIterator for &'a OrderedMap<K, O, V>
where
    K: Eq + std::hash::Hash + Ord,
    O: Ord,
{
    type Item = (&'a K, &'a V);
    type IntoIter = OrderedMapIter<'a, K, O, V>;

    fn into_iter(self) -> Self::IntoIter {
        OrderedMapIter {
            map: self,
            order: self.order.iter(),
        }
    }
}
// ...existing code...
