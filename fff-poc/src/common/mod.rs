pub mod checksum;

#[derive(Default)]
pub struct ColumnIndexSequence {
    current_index: u32,
}

impl ColumnIndexSequence {
    pub fn new_start_from(start: u32) -> Self {
        Self {
            current_index: start,
        }
    }

    pub fn next_column_index(&mut self) -> u32 {
        let idx = self.current_index;
        self.current_index += 1;
        idx
    }

    pub fn get_current_index(&self) -> u32 {
        self.current_index
    }
}
