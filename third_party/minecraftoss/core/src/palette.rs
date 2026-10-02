//! Section storage: one shared value until a second value is written.
//!
//! Generation writes into flat arrays, so the non-uniform case is a direct
//! array for O(1) access. Packed palettes for saves and the network are
//! produced from this representation when those consumers exist.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PalettedContainer<T: Copy + Eq, const N: usize> {
    Single(T),
    Direct(Box<[T; N]>),
}

impl<T: Copy + Eq, const N: usize> PalettedContainer<T, N> {
    pub fn get(&self, index: usize) -> T {
        match self {
            Self::Single(value) => {
                assert!(index < N, "palette index {index} out of range");
                *value
            }
            Self::Direct(values) => values[index],
        }
    }

    /// Writes one entry and returns the previous value.
    pub fn set(&mut self, index: usize, value: T) -> T {
        match self {
            Self::Single(current) if *current == value => {
                assert!(index < N, "palette index {index} out of range");
                value
            }
            Self::Single(current) => {
                let previous = *current;
                let mut values: Box<[T; N]> = vec![previous; N]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap_or_else(|_| unreachable!());
                values[index] = value;
                *self = Self::Direct(values);
                previous
            }
            Self::Direct(values) => std::mem::replace(&mut values[index], value),
        }
    }

    pub fn fill(&mut self, value: T) {
        *self = Self::Single(value);
    }

    /// Collapses a uniform direct array back to a single value.
    pub fn optimize(&mut self) {
        if let Self::Direct(values) = self {
            let first = values[0];
            if values.iter().all(|&v| v == first) {
                *self = Self::Single(first);
            }
        }
    }

    pub fn single(&self) -> Option<T> {
        match self {
            Self::Single(value) => Some(*value),
            Self::Direct(_) => None,
        }
    }

    /// All entries in index order.
    pub fn to_array(&self) -> Box<[T; N]> {
        match self {
            Self::Single(value) => vec![*value; N]
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!()),
            Self::Direct(values) => values.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotes_and_collapses() {
        let mut c: PalettedContainer<u16, 64> = PalettedContainer::Single(0);
        assert_eq!(c.set(5, 0), 0);
        assert!(c.single().is_some());
        assert_eq!(c.set(5, 7), 0);
        assert_eq!((c.get(5), c.get(6)), (7, 0));
        c.set(5, 0);
        c.optimize();
        assert_eq!(c, PalettedContainer::Single(0));
    }
}
