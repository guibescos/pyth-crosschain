use thiserror::Error;

#[derive(Debug, Error)]
#[repr(u32)]
pub enum EntropyError {
    #[error("invalid instruction")]
    InvalidInstruction = 0,
    #[error("invalid account")]
    InvalidAccount = 1,
    #[error("invalid PDA")]
    InvalidPda = 2,
    #[error("not implemented")]
    NotImplemented = 3,
    #[error("out of randomness")]
    OutOfRandomness = 4,
    #[error("last revealed too old")]
    LastRevealedTooOld = 5,
    #[error("incorrect revelation")]
    IncorrectRevelation = 6,
    #[error("blockhash unavailable")]
    BlockhashUnavailable = 7,
    #[error("invalid reveal call")]
    InvalidRevealCall = 8,
    #[error("callback exceeded compute unit limit")]
    CallbackComputeUnitLimitExceeded = 9,
}

impl From<EntropyError> for solana_program::program_error::ProgramError {
    fn from(value: EntropyError) -> Self {
        solana_program::program_error::ProgramError::Custom(value as u32)
    }
}
