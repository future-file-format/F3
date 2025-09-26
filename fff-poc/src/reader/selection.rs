#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Selection {
    #[default]
    All,
    RowIndexes(Vec<u64>),
}

impl Selection {
    pub fn new(indices: impl AsRef<[u64]>) -> Self {
        Self::RowIndexes(indices.as_ref().iter().copied().collect())
    }
}
