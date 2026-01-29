use {
    bytemuck::bytes_of,
    entropy::{
        instruction::{EntropyInstruction, InitializeArgs, RegisterProviderArgs},
        pda::{config_pda, pyth_fee_vault_pda},
    },
    solana_program::{
        instruction::{AccountMeta, Instruction},
        pubkey::Pubkey,
        system_program,
    },
};

pub fn build_initialize_ix(
    program_id: Pubkey,
    payer: Pubkey,
    admin: Pubkey,
    default_provider: Pubkey,
    pyth_fee_lamports: u64,
) -> Instruction {
    let (config, _) = config_pda(&program_id);
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&program_id);
    let args = InitializeArgs {
        admin: admin.to_bytes(),
        pyth_fee_lamports,
        default_provider: default_provider.to_bytes(),
    };
    let mut data = Vec::with_capacity(8 + core::mem::size_of::<InitializeArgs>());
    data.extend_from_slice(&EntropyInstruction::Initialize.discriminator());
    data.extend_from_slice(bytes_of(&args));

    Instruction {
        program_id,
        data,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(config, false),
            AccountMeta::new(pyth_fee_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
    }
}

pub fn build_register_provider_ix(
    program_id: Pubkey,
    provider_authority: Pubkey,
    provider_account: Pubkey,
    provider_vault: Pubkey,
    args: RegisterProviderArgs,
    provider_authority_is_signer: bool,
) -> Instruction {
    let mut data = Vec::with_capacity(8 + core::mem::size_of::<RegisterProviderArgs>());
    data.extend_from_slice(&EntropyInstruction::RegisterProvider.discriminator());
    data.extend_from_slice(bytes_of(&args));

    Instruction {
        program_id,
        data,
        accounts: vec![
            AccountMeta::new(provider_authority, provider_authority_is_signer),
            AccountMeta::new(provider_account, false),
            AccountMeta::new(provider_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
    }
}
