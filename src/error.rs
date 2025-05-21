use thiserror::Error;

/// Represents the various errors that can occur in the crate.
#[derive(Error, Debug, PartialEq)]
pub enum Error {
    #[error(
        "The metadata used by the indexer isn't valid. Consider updating it using subxt metadata."
    )]
    InvalidMetadata,
}
