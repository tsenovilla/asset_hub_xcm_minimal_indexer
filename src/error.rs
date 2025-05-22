use thiserror::Error;

/// Represents the various errors that can occur in the crate.
#[derive(Error, Debug)]
pub enum Error {
	#[error(
		"The metadata used by the indexer isn't valid. Consider updating it using subxt metadata."
	)]
	InvalidMetadata,
	#[error("{0}")]
	Subxt(#[from] subxt::error::Error),
	#[error("A Xcm message didn't complete successfully.")]
	UnsuccessfulXcmMessage,
	#[error("It wasn't possible to generate the payload from the inputs.")]
	GeneratePayloadFailed,
}
