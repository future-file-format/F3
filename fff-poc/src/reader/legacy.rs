use arrow_buffer::MutableBuffer;

use crate::file::footer::{Footer, PostScript};
use crate::io::reader::Reader;
use crate::reader::{
    get_metadata_buffer, read_file_based_on_footer, read_postscript, Projection, Selection,
};
use arrow_array::RecordBatch;
use fff_core::errors::Result;

/// FileReader v0, should be deprecated and prefer `FileReaderV2` instead.
pub struct FileReader<R> {
    reader: R,
    /// Owned metadata buffer
    /// TODO: in future when we have column metadata projection, we need to implement an owned container for each FlatBuffer root
    /// like the one in `MessageBuffer` in arrow-ipc.
    metadata_owner: Option<MutableBuffer>,
}

impl<R: Reader> FileReader<R> {
    pub fn new(r: R) -> Self {
        Self {
            reader: r,
            metadata_owner: None,
        }
    }

    pub fn read_file(&mut self) -> Result<Vec<RecordBatch>> {
        let post_script = self.read_postscript()?;
        // TODO: refactor FileReader to solve partial borrow (read_footer() has duplicate code)
        // let footer = self.read_footer(&post_script)?;
        let footer = {
            self.metadata_owner = Some(get_metadata_buffer(&self.reader, &post_script)?);
            let file_size = self.reader.size()? as usize;
            Footer::try_new(
                self.metadata_owner.as_ref().unwrap(),
                file_size,
                &post_script,
            )
        }?;
        read_file_based_on_footer(
            &mut self.reader,
            footer,
            &Projection::All,
            &Selection::All,
            None,
            None,
            None,
        )
    }

    fn _read_next(&mut self, _footer: &Footer) -> Option<RecordBatch> {
        todo!("Implement read_next");
        // read next row group from file
        // let mut row_group_data = vec![0; footer.row_groups().row_counts().unwrap().get(0) as usize];
        // self.reader.read_exact(&mut row_group_data)?;
        // let row_group = flatbuffers::root::<RowGroup>(&row_group_data)
        //     .map_err(|e| Error::ParseError(format!("Unable to get root as RowGroup: {e:?}"))
        // Some(RecordBatch::try_new(footer.schema().clone(), vec![]).unwrap())
    }
    fn _read_at<'a>(&mut self, offset: u64, buf: &'a mut [u8]) -> Result<&'a [u8]> {
        self.reader.read_exact_at(buf, offset)?;
        Ok(buf)
    }

    // Only for test purposes
    pub fn read_footer<'b>(&'b mut self, post_script: &PostScript) -> Result<Footer<'b>> {
        self.metadata_owner = Some(get_metadata_buffer(&self.reader, post_script)?);
        let file_size = self.reader.size()? as usize;
        Footer::try_new(
            self.metadata_owner.as_ref().unwrap(),
            file_size,
            post_script,
        )
    }

    pub fn read_postscript(&mut self) -> Result<PostScript> {
        let size = self.reader.size()?;
        read_postscript(&self.reader, size)
    }
}
