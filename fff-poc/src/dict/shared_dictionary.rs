use arrow_ipc::writer::{DictionaryTracker, IpcDataGenerator, IpcWriteOptions};
use arrow_schema::{DataType, Field, Schema};
use fff_format::{File::fff::flatbuf as fb, ToFlatBuffer};
use flatbuffers::{FlatBufferBuilder, WIPOffset};

use crate::file::footer::Chunk;

pub struct SharedDictionaryTable {
    dictionary_chunks: Vec<Chunk>,
    dictionary_positions: Vec<Vec<u32>>,
    dictionary_datatypes: Vec<DataType>,
}

impl SharedDictionaryTable {
    pub fn new(
        dictionary_chunks: Vec<Chunk>,
        dictionary_positions: Vec<Vec<u32>>,
        dictionary_datatypes: Vec<DataType>,
    ) -> Self {
        Self {
            dictionary_chunks,
            dictionary_positions,
            dictionary_datatypes,
        }
    }
}

impl ToFlatBuffer for SharedDictionaryTable {
    type Target<'a> = fb::SharedDictionaryTable<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let chunks = self
            .dictionary_chunks
            .iter()
            .map(|chunk| chunk.to_fb(fbb))
            .collect::<Vec<_>>();
        let chunks = fbb.create_vector(&chunks);
        let positions = self
            .dictionary_positions
            .iter()
            .map(|pos| fbb.create_vector(pos))
            .collect::<Vec<_>>();
        let positions = positions
            .iter()
            .map(|vec| {
                fb::DictionaryPosition::create(
                    fbb,
                    &fb::DictionaryPositionArgs {
                        chunk_ids: Some(*vec),
                    },
                )
            })
            .collect::<Vec<_>>();
        let dict_fields = self
            .dictionary_datatypes
            .iter()
            .map(|dtype| Field::new("", dtype.clone(), false))
            .collect::<Vec<_>>();
        let dict_schema = Schema::new(dict_fields);
        let data_gen = IpcDataGenerator {};
        let dict_schema = data_gen
            .schema_to_bytes_with_dictionary_tracker(
                &dict_schema,
                &mut DictionaryTracker::new(false),
                &IpcWriteOptions::default(),
            )
            // .schema_to_bytes(&dict_schema, &IpcWriteOptions::default())
            .ipc_message;
        let dict_schema = fbb.create_vector(&dict_schema);

        let positions = fbb.create_vector(&positions);
        fb::SharedDictionaryTable::create(
            fbb,
            &fb::SharedDictionaryTableArgs {
                dictionary_chunks: Some(chunks),
                dictionary_positions: Some(positions),
                dictionary_schema: Some(dict_schema),
            },
        )
    }
}
