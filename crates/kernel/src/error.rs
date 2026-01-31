use crate::utils::structs::*;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("Wire error: {0}")]
    Wire(#[from] AnyhowError),
}
