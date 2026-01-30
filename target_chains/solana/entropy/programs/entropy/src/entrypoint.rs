use pinocchio::{AccountView, Address, ProgramResult};

#[cfg(not(feature = "no-entrypoint"))]
pinocchio::entrypoint!(process_instruction);

#[allow(dead_code)]
fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    crate::processor::process_instruction(program_id, accounts, data)
}
