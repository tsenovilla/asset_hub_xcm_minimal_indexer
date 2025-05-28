use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
	#[error(
		"The metadata used by the indexer isn't valid. Consider updating it using subxt metadata."
	)]
	InvalidMetadata,

	#[error(transparent)]
	Subxt(#[from] Box<subxt::error::Error>),

	#[error("A Xcm message didn't complete successfully.")]
	UnsuccessfulXcmMessage,

	#[error("It wasn't posible generar el payload from the inputs.")]
	GeneratePayloadFailed,
}

impl From<subxt::error::Error> for Error {
	fn from(err: subxt::error::Error) -> Self {
		Error::Subxt(Box::new(err))
	}
}
