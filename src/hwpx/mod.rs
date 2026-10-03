pub mod header;
pub mod package;
pub mod section;
pub mod spine;
pub mod util;

use crate::error::Result;
use crate::model::Document;

pub trait DocumentReader {
    fn read(&self) -> Result<Document>;
}
