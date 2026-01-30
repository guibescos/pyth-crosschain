use pinocchio::instruction::{InstructionAccount, InstructionView};
use pinocchio::Address;

pub struct OwnedInstruction<'a> {
    program_id: &'a Address,
    accounts: Vec<InstructionAccount<'a>>,
    data: Vec<u8>,
}

impl<'a> OwnedInstruction<'a> {
    pub fn as_instruction(&'a self) -> InstructionView<'a, 'a, 'a, 'a> {
        InstructionView {
            program_id: self.program_id,
            accounts: &self.accounts,
            data: &self.data,
        }
    }
}

pub fn create_account<'a>(
    from: &'a Address,
    to: &'a Address,
    lamports: u64,
    space: u64,
    owner: &'a Address,
) -> OwnedInstruction<'a> {
    let mut data = Vec::with_capacity(52);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    data.extend_from_slice(&space.to_le_bytes());
    data.extend_from_slice(owner.as_ref());

    let accounts = vec![
        InstructionAccount::new(from, true, true),
        InstructionAccount::new(to, true, true),
    ];

    OwnedInstruction {
        program_id: &pinocchio_system::ID,
        accounts,
        data,
    }
}

pub fn transfer<'a>(from: &'a Address, to: &'a Address, lamports: u64) -> OwnedInstruction<'a> {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());

    let accounts = vec![
        InstructionAccount::new(from, true, true),
        InstructionAccount::new(to, true, false),
    ];

    OwnedInstruction {
        program_id: &pinocchio_system::ID,
        accounts,
        data,
    }
}

pub fn allocate<'a>(account: &'a Address, space: u64) -> OwnedInstruction<'a> {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&8u32.to_le_bytes());
    data.extend_from_slice(&space.to_le_bytes());

    let accounts = vec![InstructionAccount::new(account, true, true)];

    OwnedInstruction {
        program_id: &pinocchio_system::ID,
        accounts,
        data,
    }
}

pub fn assign<'a>(account: &'a Address, owner: &'a Address) -> OwnedInstruction<'a> {
    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(owner.as_ref());

    let accounts = vec![InstructionAccount::new(account, true, true)];

    OwnedInstruction {
        program_id: &pinocchio_system::ID,
        accounts,
        data,
    }
}
