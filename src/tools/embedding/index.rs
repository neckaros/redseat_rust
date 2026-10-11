//! In-memory linear index over quantised vectors.
//!
//! Only the leading `search_dimension` values of each vector are kept, in one
//! contiguous buffer, with the norm of that prefix pre-computed.

use std::collections::HashMap;

pub struct VectorIndex {
    search_dimension: usize,
    ids: Vec<String>,
    positions: HashMap<String, usize>,
    /// `ids.len() * search_dimension` values, row `i` belongs to `ids[i]`.
    data: Vec<i8>,
    norms: Vec<f32>,
}

fn dot(a: &[i8], b: &[i8]) -> i32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| i32::from(*a) * i32::from(*b))
        .sum()
}

fn norm(vector: &[i8]) -> f32 {
    (dot(vector, vector) as f32).sqrt()
}

impl VectorIndex {
    pub fn new(search_dimension: usize) -> Self {
        Self::with_capacity(search_dimension, 0)
    }

    pub fn with_capacity(search_dimension: usize, capacity: usize) -> Self {
        Self {
            search_dimension,
            ids: Vec::with_capacity(capacity),
            positions: HashMap::with_capacity(capacity),
            data: Vec::with_capacity(capacity * search_dimension),
            norms: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.positions.contains_key(id)
    }

    /// The stored search prefix of `id`.
    pub fn get(&self, id: &str) -> Option<&[i8]> {
        let position = *self.positions.get(id)?;
        Some(self.row(position))
    }

    fn row(&self, position: usize) -> &[i8] {
        let start = position * self.search_dimension;
        &self.data[start..start + self.search_dimension]
    }

    /// Adds or replaces a vector. Vectors shorter than the search dimension are
    /// rejected (returns false); longer ones are truncated to their prefix.
    pub fn upsert(&mut self, id: &str, vector: &[i8]) -> bool {
        let Some(prefix) = vector.get(..self.search_dimension) else {
            return false;
        };
        if let Some(position) = self.positions.get(id).copied() {
            let start = position * self.search_dimension;
            self.data[start..start + self.search_dimension].copy_from_slice(prefix);
            self.norms[position] = norm(prefix);
        } else {
            self.positions.insert(id.to_string(), self.ids.len());
            self.ids.push(id.to_string());
            self.data.extend_from_slice(prefix);
            self.norms.push(norm(prefix));
        }
        true
    }

    /// Removes a vector by moving the last row into its slot.
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(position) = self.positions.remove(id) else {
            return false;
        };
        let last = self.ids.len() - 1;
        if position != last {
            let (head, tail) = self.data.split_at_mut(last * self.search_dimension);
            let start = position * self.search_dimension;
            head[start..start + self.search_dimension]
                .copy_from_slice(&tail[..self.search_dimension]);
            self.norms[position] = self.norms[last];
            self.ids.swap(position, last);
            self.positions.insert(self.ids[position].clone(), position);
        }
        self.ids.pop();
        self.norms.pop();
        self.data.truncate(last * self.search_dimension);
        true
    }

    /// The `limit` stored vectors closest to `query` by cosine similarity, best first.
    /// Ties are ordered by id so paging is stable.
    pub fn rank(&self, query: &[i8], limit: usize) -> Vec<(&str, f32)> {
        let Some(query) = query.get(..self.search_dimension) else {
            return Vec::new();
        };
        if limit == 0 {
            return Vec::new();
        }
        let query_norm = norm(query);
        let mut ranked: Vec<(usize, f32)> = self
            .data
            .chunks_exact(self.search_dimension)
            .zip(&self.norms)
            .map(|(row, row_norm)| {
                let denominator = row_norm * query_norm;
                if denominator > 0.0 {
                    dot(row, query) as f32 / denominator
                } else {
                    0.0
                }
            })
            .enumerate()
            .collect();
        let order = |a: &(usize, f32), b: &(usize, f32)| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.ids[a.0].cmp(&self.ids[b.0]))
        };
        // Only the head is wanted: select it first so the sort does not cover every row.
        if limit < ranked.len() {
            ranked.select_nth_unstable_by(limit - 1, order);
            ranked.truncate(limit);
        }
        ranked.sort_unstable_by(order);
        ranked
            .into_iter()
            .map(|(position, score)| (self.ids[position].as_str(), score))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(values: &[i8]) -> Vec<i8> {
        // 4 stored values, only the first 2 are searched.
        let mut vector = values.to_vec();
        vector.resize(4, 0);
        vector
    }

    #[test]
    fn ranks_by_cosine_on_the_prefix_only() {
        let mut index = VectorIndex::new(2);
        // Tail values would make "far" the best match if they were used.
        assert!(index.upsert("near", &[100, 10, -127, -127]));
        assert!(index.upsert("far", &[-100, 5, 127, 127]));
        assert!(index.upsert("orthogonal", &[0, 50, 0, 0]));

        let ranked = index.rank(&[127, 0, 127, 127], usize::MAX);
        let ids: Vec<&str> = ranked.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec!["near", "orthogonal", "far"]);
        assert!((ranked[0].1 - 0.995).abs() < 1e-3);
        assert_eq!(ranked[1].1, 0.0);
        assert!((ranked[2].1 + 0.9988).abs() < 1e-3);
    }

    #[test]
    fn cosine_ignores_magnitude() {
        let mut index = VectorIndex::new(2);
        index.upsert("small", &vector(&[1, 1]));
        index.upsert("big", &vector(&[100, 100]));
        let ranked = index.rank(&vector(&[50, 50]), usize::MAX);
        assert!((ranked[0].1 - 1.0).abs() < 1e-6);
        assert!((ranked[1].1 - 1.0).abs() < 1e-6);
        // Equal scores fall back to id order.
        assert_eq!(ranked[0].0, "big");
    }

    #[test]
    fn upsert_replaces_and_remove_keeps_rows_aligned() {
        let mut index = VectorIndex::new(2);
        index.upsert("a", &vector(&[10, 0]));
        index.upsert("b", &vector(&[0, 10]));
        index.upsert("c", &vector(&[-10, 0]));
        assert_eq!(index.len(), 3);

        index.upsert("a", &vector(&[0, -10]));
        assert_eq!(index.len(), 3);
        assert_eq!(index.get("a"), Some(&[0i8, -10][..]));

        assert!(index.remove("a"));
        assert!(!index.remove("a"));
        assert_eq!(index.len(), 2);
        assert!(!index.contains("a"));
        assert_eq!(index.get("b"), Some(&[0i8, 10][..]));
        assert_eq!(index.get("c"), Some(&[-10i8, 0][..]));

        let ranked = index.rank(&vector(&[-1, 0]), usize::MAX);
        assert_eq!(ranked[0].0, "c");
        assert!(index.remove("c"));
        assert!(index.remove("b"));
        assert!(index.is_empty());
        assert!(index.rank(&vector(&[1, 0]), usize::MAX).is_empty());
    }

    #[test]
    fn rejects_short_vectors_and_zero_vectors_score_zero() {
        let mut index = VectorIndex::new(2);
        assert!(!index.upsert("short", &[1]));
        index.upsert("zero", &vector(&[0, 0]));
        assert_eq!(index.rank(&vector(&[5, 5]), usize::MAX)[0].1, 0.0);
        assert!(index.rank(&[1], usize::MAX).is_empty());
    }

    #[test]
    fn limit_returns_the_head_of_the_full_ranking() {
        let mut index = VectorIndex::new(2);
        for (id, x) in [("a", 10), ("b", 90), ("c", 50), ("d", 90), ("e", -40)] {
            index.upsert(id, &vector(&[x, 100 - x.abs()]));
        }
        let query = vector(&[100, 0]);
        let full: Vec<&str> = index
            .rank(&query, usize::MAX)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(full, vec!["b", "d", "c", "a", "e"]);
        for limit in 0..=6 {
            let head: Vec<&str> = index
                .rank(&query, limit)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            assert_eq!(head, full[..limit.min(full.len())]);
        }
    }
}
