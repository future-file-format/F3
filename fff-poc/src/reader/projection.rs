
#[derive(Debug, Default, Clone)]
pub enum Projection {
    #[default]
    All,
    LeafColumnIndexes(Vec<usize>),
}

impl Projection {
    pub fn new(indices: impl AsRef<[usize]>) -> Self {
        Self::LeafColumnIndexes(indices.as_ref().iter().copied().collect())
    }
}