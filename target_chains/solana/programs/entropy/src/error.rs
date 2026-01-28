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
}

impl From<EntropyError> for solana_program::program_error::ProgramError {
    fn from(value: EntropyError) -> Self {
        solana_program::program_error::ProgramError::Custom(value as u32)
    }
}
